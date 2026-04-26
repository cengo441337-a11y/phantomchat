import { KeyboardEvent, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import type { MlsMemberRef } from "../types";
import MentionPopover from "./MentionPopover";

interface Props {
  activeLabel: string | null;
  onSend: (body: string) => Promise<void> | void;
  /// File-send handler. Called with the absolute path returned by the
  /// Tauri file picker. Optional so callers that don't yet support file
  /// transfer (e.g. tests, future variants) can omit the prop.
  onSendFile?: (filePath: string) => Promise<void> | void;
  /// Debounced typing-ping callback (1.5s leading-edge throttle is
  /// applied INSIDE this component). Backend wraps the call in a
  /// sealed-sender `TYPN-1:` envelope.
  onTypingPing?: (contactLabel: string) => void;
  /// Label to render in the "<label> is typing…" pill above the input,
  /// or `null` when no peer is currently typing to the active contact.
  typingFromLabel?: string | null;
  /// MLS group directory used to populate the @-mention auto-complete
  /// popover. `undefined` (or empty) suppresses the popover entirely —
  /// passed as `undefined` for 1:1 chats since mentions are meaningless
  /// outside of a multi-party group.
  mlsDirectory?: MlsMemberRef[];
  /// When set, the input is in "reply mode" — sending routes through the
  /// REPL-1: envelope path (via `onSendReply` rather than `onSend`) and a
  /// magenta-tinted quote block sits above the input bar showing what the
  /// user is replying to. `null` = normal compose mode.
  replyingTo?: { msg_id: string; preview: string } | null;
  /// Cancel-reply handler — called when the user clicks the X on the
  /// quote block to drop back into normal compose mode.
  onCancelReply?: () => void;
  /// Reply-send handler invoked instead of `onSend` whenever
  /// `replyingTo` is set. Receives the quoted msg_id + preview so the
  /// parent can route to the backend `send_reply` command.
  onSendReply?: (
    body: string,
    inReplyToMsgId: string,
    quotedPreview: string,
  ) => Promise<void> | void;
}

/// Leading-edge cooldown in ms — matches backend `TYPING_TTL_SECS = 5`
/// minus a healthy buffer so the receiver's pill never expires between
/// pings while the user is actively typing.
const TYPING_PING_COOLDOWN_MS = 1500;

export default function InputBar({
  activeLabel,
  onSend,
  onSendFile,
  onTypingPing,
  typingFromLabel,
  mlsDirectory,
  replyingTo,
  onCancelReply,
  onSendReply,
}: Props) {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [uploading, setUploading] = useState(false);
  // Track focus locally so we can dim the blinking cursor when the input
  // is unfocused, per spec. We don't lift this state — it's purely cosmetic.
  const [focused, setFocused] = useState(false);
  /// Timestamp (ms) of the most recent typing-ping we fired. Used to
  /// gate subsequent keystrokes so we don't flood the relay with one
  /// envelope per keystroke. Reset on contact-switch via the inline
  /// check inside `maybeFireTypingPing`.
  const lastPingMsRef = useRef<number>(0);
  const lastPingLabelRef = useRef<string | null>(null);

  /// Mention auto-complete state. `mentionStart` is the index of the `@`
  /// in `text` that opened the popover — `null` when no popover is open.
  /// `mentionPrefix` is the substring after the `@` up to the caret;
  /// the candidate filter narrows by case-insensitive prefix match.
  /// `mentionActiveIdx` tracks the keyboard-highlighted row.
  const inputRef = useRef<HTMLInputElement>(null);
  const [mentionStart, setMentionStart] = useState<number | null>(null);
  const [mentionPrefix, setMentionPrefix] = useState("");
  const [mentionActiveIdx, setMentionActiveIdx] = useState(0);

  const mentionCandidates = useMemo(() => {
    if (mentionStart === null || !mlsDirectory) return [];
    const prefix = mentionPrefix.toLowerCase();
    return mlsDirectory
      .filter(m => m.label.toLowerCase().startsWith(prefix))
      .slice(0, 8);
  }, [mentionStart, mentionPrefix, mlsDirectory]);

  /// Re-derive the mention popover state from the current input value +
  /// caret position. Called from `onChange` and arrow-key handlers so
  /// editing the prefix narrows / dismisses the popover live.
  function recomputeMentionState(value: string, caret: number) {
    if (!mlsDirectory || mlsDirectory.length === 0) {
      setMentionStart(null);
      return;
    }
    // Walk backwards from the caret looking for the most recent `@` at a
    // word boundary. Stop at whitespace — once we cross one, there's no
    // active mention to complete.
    let i = caret - 1;
    while (i >= 0) {
      const ch = value[i];
      if (ch === "@") {
        const prev = i > 0 ? value[i - 1] : "";
        const atWordBoundary = i === 0 || /\s/.test(prev);
        if (!atWordBoundary) {
          setMentionStart(null);
          return;
        }
        const prefix = value.slice(i + 1, caret);
        // Reject if the prefix already contains anything non-label-y.
        if (/[^A-Za-z0-9_.-]/.test(prefix)) {
          setMentionStart(null);
          return;
        }
        setMentionStart(i);
        setMentionPrefix(prefix);
        setMentionActiveIdx(0);
        return;
      }
      if (/\s/.test(ch)) {
        setMentionStart(null);
        return;
      }
      i -= 1;
    }
    setMentionStart(null);
  }

  /// Insert the picked label at `mentionStart`, replacing the partial
  /// `@<prefix>`. Trailing space is convenience — the user almost always
  /// wants to type more after a mention.
  function selectMention(label: string) {
    if (mentionStart === null) return;
    const before = text.slice(0, mentionStart);
    const after = text.slice(mentionStart + 1 + mentionPrefix.length);
    const replacement = `@${label} `;
    const next = before + replacement + after;
    setText(next);
    setMentionStart(null);
    setMentionPrefix("");
    // Restore the caret right after the inserted mention.
    const caret = before.length + replacement.length;
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus();
      try {
        el.setSelectionRange(caret, caret);
      } catch {
        /* setSelectionRange throws on certain input types — ignore. */
      }
    });
  }

  function maybeFireTypingPing() {
    if (!onTypingPing || !activeLabel) return;
    const now = Date.now();
    // If the active contact changed, reset the cooldown so the first
    // keystroke after a switch always fires (otherwise the previous
    // contact's cooldown could swallow it).
    if (lastPingLabelRef.current !== activeLabel) {
      lastPingLabelRef.current = activeLabel;
      lastPingMsRef.current = 0;
    }
    if (now - lastPingMsRef.current < TYPING_PING_COOLDOWN_MS) return;
    lastPingMsRef.current = now;
    onTypingPing(activeLabel);
  }

  async function fire() {
    const body = text.trim();
    if (!body || sending) return;
    setSending(true);
    try {
      // When reply mode is active, route through the REPL-1: handler so
      // the peer's incoming row carries the quote inline. Falls back to
      // normal `onSend` if the parent didn't wire `onSendReply` yet.
      if (replyingTo && onSendReply) {
        await onSendReply(body, replyingTo.msg_id, replyingTo.preview);
      } else {
        await onSend(body);
      }
      setText("");
    } finally {
      setSending(false);
    }
  }

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    // Mention popover claims arrow / Enter / Tab / Escape when open with
    // at least one candidate. Falls through to send-on-enter otherwise.
    if (mentionStart !== null && mentionCandidates.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setMentionActiveIdx(i => (i + 1) % mentionCandidates.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setMentionActiveIdx(i =>
          (i - 1 + mentionCandidates.length) % mentionCandidates.length,
        );
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        const pick = mentionCandidates[mentionActiveIdx];
        if (pick) selectMention(pick.label);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setMentionStart(null);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void fire();
    }
  }

  /// Paperclip handler — pops the OS file picker, then forwards the absolute
  /// path to the parent's `onSendFile`. We disable the input briefly during
  /// upload so a user can't queue a text send mid-transfer.
  async function pickAndSendFile() {
    if (!activeLabel || uploading || !onSendFile) return;
    setUploading(true);
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        title: t("input_bar.file_picker_title", { label: activeLabel }),
      });
      // Tauri returns `null` on cancel; on success either a string (legacy)
      // or a `{ path }`-shaped object depending on plugin version. Coerce
      // to the absolute path string regardless of the wrapper shape.
      if (selected === null || selected === undefined) return;
      const filePath =
        typeof selected === "string"
          ? selected
          : Array.isArray(selected)
          ? null // multiple === false should never produce an array, but be defensive
          : (selected as { path?: string }).path ?? null;
      if (!filePath) return;
      await onSendFile(filePath);
    } finally {
      setUploading(false);
    }
  }

  const hint = activeLabel
    ? t("input_bar.placeholder_target", { label: activeLabel })
    : t("input_bar.placeholder_no_contact");

  return (
    <div className="border-t border-dim-green/40 bg-bg-panel/85 backdrop-blur-sm pc-pane">
      {/* Reply quote block — sits above the typing pill so the user
          sees what they're quoting before they hit send. X button drops
          back to normal compose mode. */}
      {replyingTo && (
        <div className="mx-4 mt-2 flex items-start gap-2 text-xs px-2 py-1 rounded bg-neon-magenta/10 border-l-2 border-neon-magenta">
          <span className="text-neon-magenta font-bold">{"↪"}</span>
          <span className="flex-1 text-soft-grey italic truncate">
            {replyingTo.preview}
          </span>
          <button
            onClick={() => onCancelReply?.()}
            className="text-soft-grey hover:text-neon-magenta transition-colors px-1"
            aria-label={t("messages.reply.cancel_aria")}
            title={t("messages.reply.cancel_title")}
          >
            {"✕"}
          </button>
        </div>
      )}
      {/* Typing pill — sits above the input. Empty span keeps layout
          stable so the input doesn't jump up/down as the pill flickers. */}
      <div className="px-4 pt-1.5 h-5 text-xs leading-tight">
        {typingFromLabel ? (
          <span className="text-cyber-cyan animate-pulse">
            {t("input_bar.typing_pill", { label: typingFromLabel, defaultValue: "{{label}} is typing…" })}
          </span>
        ) : (
          <span aria-hidden="true">&nbsp;</span>
        )}
      </div>
      <div className="flex items-center gap-2 px-4 pb-3 pt-1">
      {/* Paperclip — neon-magenta glow on hover, dim when no contact /
          mid-upload. Title surfaces the 5 MiB cap so the user is warned
          before they hit the file-picker. */}
      <button
        onClick={() => void pickAndSendFile()}
        disabled={!activeLabel || uploading || !onSendFile}
        className="text-cyber-cyan hover:text-neon-magenta hover:pc-brand-glow-magenta disabled:opacity-40 transition-colors text-lg leading-none px-1"
        title={
          activeLabel
            ? t("input_bar.attach_title", { label: activeLabel })
            : t("input_bar.attach_title_no_contact")
        }
        aria-label={t("input_bar.attach_aria")}
      >
        {"\u{1F4CE}"}
      </button>

      {/* Wave 11B — voice-record affordance placeholder. Disabled
          (recording on the desktop is a Wave 11B-extension; for v1 only
          the mobile side records, the desktop only PLAYS BACK incoming
          voice messages). The button is here so the UI affordance is
          discoverable and so the eventual record path has a visual
          anchor to attach to. */}
      <button
        type="button"
        disabled
        className="text-cyber-cyan/40 disabled:opacity-40 cursor-not-allowed text-lg leading-none px-1"
        title={t("input_bar.mic_coming_soon")}
        aria-label={t("input_bar.mic_aria")}
      >
        {"\u{1F399}"}
      </button>

      <span className="text-neon-magenta font-bold pc-brand-glow-magenta">»</span>
      {/* Wrap input so we can overlay a blinking cursor glyph at the
          end of the typed text without losing the native caret. The
          overlay is purely decorative — it sits behind the real input. */}
      <div className="relative flex-1 flex items-center">
        <input
          ref={inputRef}
          type="text"
          value={text}
          onChange={e => {
            const value = e.target.value;
            setText(value);
            const caret = e.target.selectionStart ?? value.length;
            recomputeMentionState(value, caret);
            // Leading-edge throttled typing-ping. Only fires when text
            // actually grew (so blank → typing transitions ping
            // immediately) and the cooldown has elapsed.
            if (value.length > 0) {
              maybeFireTypingPing();
            }
          }}
          onKeyUp={e => {
            // Re-check after caret movement (arrow keys) so the popover
            // dismisses if the user navigates away from the @ token.
            if (
              e.key === "ArrowLeft" ||
              e.key === "ArrowRight" ||
              e.key === "Home" ||
              e.key === "End"
            ) {
              const target = e.currentTarget;
              recomputeMentionState(target.value, target.selectionStart ?? 0);
            }
          }}
          onClick={e => {
            const target = e.currentTarget;
            recomputeMentionState(target.value, target.selectionStart ?? 0);
          }}
          onKeyDown={onKeyDown}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          disabled={!activeLabel || sending || uploading}
          placeholder={uploading ? t("input_bar.placeholder_uploading") : hint}
          className="neon-input w-full disabled:opacity-50"
          autoFocus
        />
        {mentionStart !== null && mlsDirectory && (
          <MentionPopover
            candidates={mentionCandidates}
            activeIdx={mentionActiveIdx}
            onSelect={label => selectMention(label)}
            onDismiss={() => setMentionStart(null)}
          />
        )}
        {/* Decorative blinking cursor — shown only when there's no typed
            text so it doesn't compete with the OS caret mid-typing. */}
        {!text && (
          <span
            aria-hidden="true"
            className={
              "pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 animate-cursor-blink select-none " +
              (focused ? "pc-cursor" : "pc-cursor pc-cursor-dim")
            }
          >
            ▮
          </span>
        )}
      </div>
      <button
        onClick={() => void fire()}
        disabled={!activeLabel || sending || !text.trim() || uploading}
        className="neon-button disabled:opacity-40 disabled:hover:bg-bg-elevated disabled:hover:shadow-none disabled:hover:filter-none disabled:hover:translate-y-0"
      >
        {sending ? t("input_bar.sealing_button") : t("input_bar.send_button")}
      </button>
      </div>
    </div>
  );
}
