import { KeyboardEvent, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";

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
      await onSend(body);
      setText("");
    } finally {
      setSending(false);
    }
  }

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>) {
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

      <span className="text-neon-magenta font-bold pc-brand-glow-magenta">»</span>
      {/* Wrap input so we can overlay a blinking cursor glyph at the
          end of the typed text without losing the native caret. The
          overlay is purely decorative — it sits behind the real input. */}
      <div className="relative flex-1 flex items-center">
        <input
          type="text"
          value={text}
          onChange={e => {
            setText(e.target.value);
            // Leading-edge throttled typing-ping. Only fires when text
            // actually grew (so blank → typing transitions ping
            // immediately) and the cooldown has elapsed.
            if (e.target.value.length > 0) {
              maybeFireTypingPing();
            }
          }}
          onKeyDown={onKeyDown}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          disabled={!activeLabel || sending || uploading}
          placeholder={uploading ? t("input_bar.placeholder_uploading") : hint}
          className="neon-input w-full disabled:opacity-50"
          autoFocus
        />
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
