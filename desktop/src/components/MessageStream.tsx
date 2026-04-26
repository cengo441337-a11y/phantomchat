import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import type { IncomingMessage, MsgLine, ReactionEntry } from "../types";
import { renderMarkdown } from "../lib/markdown";
import { autoLinkify } from "../lib/linkify";
import { detectMentions } from "../lib/mentions";
import EmojiPicker from "./EmojiPicker";

interface Props {
  messages: MsgLine[];
  activeLabel: string | null;
  /// Called when the user clicks the inline "bind" button on an unbound
  /// (`?<hex>`) row. Opens the bind modal in the parent.
  onBindClick?: () => void;
  /// Called when the user clicks a file row's filename / "open folder"
  /// affordance. Routes through the backend so the platform's Downloads
  /// resolution stays in one place.
  onOpenFolder?: () => void;
  /// Index (into `messages`) of a row to scroll into view + briefly
  /// pulse-highlight. Used by the search panel's "jump to result" flow.
  /// The pulse animation auto-decays via CSS — we just toggle the class
  /// on the matching row whenever the prop value changes.
  highlightedIdx?: number;
  /// Called the FIRST time an incoming row scrolls into view (and the
  /// window is focused), so the parent can fire `mark_read(msg_id, label)`
  /// over the receipt wire. Skipped if the row has no `msg_id` (legacy
  /// pre-feature persisted history rows) or if we already fired for it.
  onMarkRead?: (msgId: string, fromLabel: string) => void;
  /// Wave 8G — when the user picks a starred message in the global drawer
  /// we route through the parent so it can switch the active conversation
  /// before scrolling. Receives the contact label of the starred row.
  onSwitchConversation?: (contactLabel: string) => void;
  /// Labels eligible to be rendered as @-mention pills. Contains the
  /// local `me.label`, every active MLS group member's label, and every
  /// 1:1 contact label. Empty / undefined disables mention rendering
  /// entirely (text rows render as plain markdown + auto-links only).
  knownMentionLabels?: string[];
  /// Called when the user clicks the row toolbar's "Reply" button. The
  /// parent stashes the (msg_id, body_preview) pair in its `replyingTo`
  /// state and the InputBar shows the quote block above the input.
  onReplyTo?: (msgId: string, preview: string) => void;
  /// Called when the user picks an emoji from the inline reaction picker.
  /// Parent decides whether this is an "add" or "remove" action by
  /// inspecting whether the active user (`"you"`) already reacted.
  onReact?: (msgId: string, emoji: string, action: "add" | "remove") => void;
}

function truncateLabel(label: string): string {
  // 10-char hard cap so a long contact name doesn't blow out the column.
  if (label.length <= 10) return label.padEnd(10, " ");
  return label.slice(0, 9) + "…";
}

function humanSize(n: number): string {
  const K = 1024;
  if (n < K) return `${n} B`;
  if (n < K * K) return `${(n / K).toFixed(1)} KiB`;
  if (n < K * K * K) return `${(n / (K * K)).toFixed(1)} MiB`;
  return `${(n / (K * K * K)).toFixed(1)} GiB`;
}

/// Image MIME / extension detection for the inline-thumbnail branch. We
/// trust the backend `mime` field first (set from the wire `FileManifest`)
/// and fall back to the filename extension for legacy persisted file rows
/// that pre-date the FileMeta.mime extension.
const IMAGE_EXTS = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "bmp",
]);

function isImageFile(mime: string | undefined, filename: string): boolean {
  if (mime && mime.toLowerCase().startsWith("image/")) return true;
  const lower = filename.toLowerCase();
  const dot = lower.lastIndexOf(".");
  if (dot < 0) return false;
  return IMAGE_EXTS.has(lower.slice(dot + 1));
}

/// Format a remaining-seconds count as a coarse `<H>h <M>m` / `<M>m <S>s`
/// label for the disappearing-messages countdown next to the timestamp.
/// Negative values render as `0s` so the row briefly shows "expired" before
/// the auto-purge sweep removes it.
function humanTtl(remainingSecs: number): string {
  if (remainingSecs <= 0) return "0s";
  if (remainingSecs >= 86400) {
    const d = Math.floor(remainingSecs / 86400);
    const h = Math.floor((remainingSecs % 86400) / 3600);
    return h > 0 ? `${d}d ${h}h` : `${d}d`;
  }
  if (remainingSecs >= 3600) {
    const h = Math.floor(remainingSecs / 3600);
    const m = Math.floor((remainingSecs % 3600) / 60);
    return m > 0 ? `${h}h ${m}m` : `${h}h`;
  }
  if (remainingSecs >= 60) {
    const m = Math.floor(remainingSecs / 60);
    const s = remainingSecs % 60;
    return s > 0 ? `${m}m ${s}s` : `${m}m`;
  }
  return `${remainingSecs}s`;
}

/// Group reactions by emoji so the pill row reads `[👍 2] [❤️ 1]` instead
/// of one pill per sender. Returns the grouped list with the senders
/// preserved for the hover tooltip.
function groupReactions(reactions: ReactionEntry[]): Array<{
  emoji: string;
  count: number;
  senders: string[];
}> {
  const map = new Map<string, string[]>();
  for (const r of reactions) {
    const senders = map.get(r.emoji) ?? [];
    senders.push(r.sender_label);
    map.set(r.emoji, senders);
  }
  return Array.from(map.entries()).map(([emoji, senders]) => ({
    emoji,
    count: senders.length,
    senders,
  }));
}

export default function MessageStream({
  messages,
  activeLabel,
  onBindClick,
  onOpenFolder,
  highlightedIdx,
  onMarkRead,
  onSwitchConversation,
  knownMentionLabels,
  onReplyTo,
  onReact,
}: Props) {
  const { t } = useTranslation();
  const bottomRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  /// Index of the row whose emoji picker is currently open. `null` =
  /// no popover. Closing happens on outside click + Escape inside the
  /// EmojiPicker component itself.
  const [pickerForIdx, setPickerForIdx] = useState<number | null>(null);
  /// Tick-tock for live disappearing-messages countdowns. We update once
  /// per second so the "(in 4h 12m)" label re-renders without forcing
  /// the parent's `messages` reducer to fire on every tick. Bumped once
  /// per second only when at least one row actually has an `expires_at`
  /// to avoid pointless re-renders on chats without disappearing on.
  const [, setNowTick] = useState(0);
  useEffect(() => {
    const hasExpiring = messages.some(m => m.expires_at !== undefined);
    if (!hasExpiring) return;
    const id = window.setInterval(() => setNowTick(n => n + 1), 1000);
    return () => window.clearInterval(id);
  }, [messages]);
  /// Set of `msg_id`s we've already fired `mark_read` for. Persists across
  /// re-renders via `useRef` so a rapid scroll doesn't trigger duplicate
  /// receipts for the same incoming row.
  const readFiredRef = useRef<Set<string>>(new Set());
  const onMarkReadRef = useRef(onMarkRead);
  onMarkReadRef.current = onMarkRead;
  // Keep a stable index → message lookup for the IntersectionObserver
  // callback (which only sees the DOM node + its data attribute).
  const messagesRef = useRef<MsgLine[]>(messages);
  messagesRef.current = messages;

  // ── Wave 8G drawer state ─────────────────────────────────────────────
  // `pinnedDrawer` lists pinned messages of the active conversation.
  // `starredDrawer` lists ALL starred messages (cross-conversation).
  // Both are fetched lazily on open via the backend list helpers so the
  // drawer always reflects on-disk truth, not in-memory `messages`.
  const [pinnedDrawerOpen, setPinnedDrawerOpen] = useState(false);
  const [starredDrawerOpen, setStarredDrawerOpen] = useState(false);
  const [pinnedDrawerRows, setPinnedDrawerRows] = useState<IncomingMessage[]>([]);
  const [starredDrawerRows, setStarredDrawerRows] = useState<IncomingMessage[]>([]);

  // Per-conversation pinned/starred counters drive the header buttons —
  // computed from in-memory `messages` so they update instantly when the
  // backend `message_state_changed` event lands without re-fetching.
  const pinnedCount = useMemo(
    () => messages.filter(m => m.pinned).length,
    [messages],
  );
  const starredCount = useMemo(
    () => messages.filter(m => m.starred).length,
    [messages],
  );

  // ── IntersectionObserver: fire `mark_read` for incoming rows ──────────
  // (unchanged from Wave 6 — see git history for the full rationale).
  useEffect(() => {
    const root = containerRef.current;
    if (!root) return;
    if (typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      entries => {
        if (!document.hasFocus()) return;
        const cb = onMarkReadRef.current;
        if (!cb) return;
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          const idxStr = (entry.target as HTMLElement).dataset.msgIdx;
          if (idxStr === undefined) continue;
          const idx = Number(idxStr);
          const m = messagesRef.current[idx];
          if (!m) continue;
          if (m.kind !== "incoming") continue;
          if (!m.msg_id) continue;
          if (readFiredRef.current.has(m.msg_id)) continue;
          if (
            m.label.startsWith("?") ||
            m.label === "INBOX" ||
            m.label === "INBOX!"
          ) {
            continue;
          }
          readFiredRef.current.add(m.msg_id);
          cb(m.msg_id, m.label);
        }
      },
      { root, threshold: 0.6 },
    );
    const rows = root.querySelectorAll<HTMLElement>("[data-msg-idx]");
    rows.forEach(r => observer.observe(r));
    return () => observer.disconnect();
  }, [messages.length]);

  // Pin to the bottom whenever a new message arrives — chat-app convention.
  useEffect(() => {
    if (highlightedIdx !== undefined) return;
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, highlightedIdx]);

  // Search-panel "jump to result" target.
  useEffect(() => {
    if (highlightedIdx === undefined) return;
    const root = containerRef.current;
    if (!root) return;
    const row = root.querySelector<HTMLElement>(
      `[data-msg-idx="${highlightedIdx}"]`,
    );
    if (!row) return;
    row.scrollIntoView({ behavior: "smooth", block: "center" });
    row.classList.remove("pc-search-pulse");
    void row.offsetWidth;
    row.classList.add("pc-search-pulse");
    const tt = window.setTimeout(() => {
      row.classList.remove("pc-search-pulse");
    }, 1600);
    return () => window.clearTimeout(tt);
  }, [highlightedIdx]);

  // ── Wave 8G: pin / star action handlers ──────────────────────────────
  // Fire-and-forget — the backend emits `message_state_changed`, which
  // App.tsx listens for + patches `messages` in place. Errors are
  // surfaced via console.warn rather than the message stream so a failed
  // pin doesn't pollute the transcript with a system row.
  function handleTogglePin(msgId: string, currentlyPinned: boolean) {
    const cmd = currentlyPinned ? "unpin_message" : "pin_message";
    void invoke(cmd, { msgId }).catch(e =>
      console.warn(`${cmd} failed:`, e),
    );
  }
  function handleToggleStar(msgId: string, currentlyStarred: boolean) {
    const cmd = currentlyStarred ? "unstar_message" : "star_message";
    void invoke(cmd, { msgId }).catch(e =>
      console.warn(`${cmd} failed:`, e),
    );
  }

  /// Open an image file in the OS's default image viewer via the
  /// `tauri-plugin-opener` plugin. We intentionally pass the absolute
  /// disk path (NOT the asset:// URL) since the plugin invokes the
  /// platform's `xdg-open` / `open` / `start` which expect a file path.
  async function openInDefaultViewer(savedPath: string) {
    try {
      await openPath(savedPath);
    } catch (e) {
      console.warn("openPath failed:", e);
    }
  }

  /// Refresh the pinned drawer rows from the backend. Called on open and
  /// any time `messages.length` changes while the drawer is mounted so a
  /// new pin lands without a manual refresh.
  useEffect(() => {
    if (!pinnedDrawerOpen) return;
    void invoke<IncomingMessage[]>("list_pinned_messages", {
      contactLabel: activeLabel,
    })
      .then(setPinnedDrawerRows)
      .catch(e => {
        console.warn("list_pinned_messages failed:", e);
        setPinnedDrawerRows([]);
      });
  }, [pinnedDrawerOpen, activeLabel, messages.length]);

  useEffect(() => {
    if (!starredDrawerOpen) return;
    void invoke<IncomingMessage[]>("list_starred_messages")
      .then(setStarredDrawerRows)
      .catch(e => {
        console.warn("list_starred_messages failed:", e);
        setStarredDrawerRows([]);
      });
  }, [starredDrawerOpen, messages.length]);

  /// Drawer click → scroll to in-stream row by msg_id. Re-uses the same
  /// `pc-search-pulse` highlight class as the SearchPanel jump path so
  /// the visual affordance stays consistent across all "jump to" flows.
  function jumpToMsgId(msgId: string) {
    const root = containerRef.current;
    if (!root) return;
    // Find the matching row index by linear scan — message arrays are
    // small enough that this is cheaper than maintaining a side index.
    const idx = messages.findIndex(m => m.msg_id === msgId);
    if (idx < 0) return;
    const row = root.querySelector<HTMLElement>(`[data-msg-idx="${idx}"]`);
    if (!row) return;
    row.scrollIntoView({ behavior: "smooth", block: "center" });
    row.classList.remove("pc-search-pulse");
    void row.offsetWidth;
    row.classList.add("pc-search-pulse");
    window.setTimeout(() => row.classList.remove("pc-search-pulse"), 1600);
  }

  const title = activeLabel ? `#${activeLabel}` : t("message_stream.title_default");

  // Stable label-set for the mention rewriter — only re-derive when the
  // upstream array's contents change. Re-deriving on every render would
  // also re-run `detectMentions` for every row's body memo below.
  const mentionLabels = useMemo(
    () => (knownMentionLabels ? knownMentionLabels.filter(Boolean) : []),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [knownMentionLabels?.join("")],
  );

  return (
    <section className="flex-1 flex flex-col bg-bg-deep/70 backdrop-blur-[1px] border-b border-dim-green/40 overflow-hidden pc-pane pc-pane-magenta">
      <div className="flex items-center justify-between px-4 py-1.5 border-b border-dim-green/30">
        <span className="text-xs text-cyber-cyan uppercase tracking-widest font-display">
          {title}
        </span>
        <div className="flex items-center gap-2">
          {activeLabel && (
            <button
              onClick={() => setPinnedDrawerOpen(o => !o)}
              className="text-[10px] uppercase tracking-wider text-soft-grey hover:text-neon-magenta border border-dim-green/40 hover:border-neon-magenta/60 px-2 py-0.5 rounded transition-colors"
              title={t("messages.pin.drawer_title")}
            >
              {"\u{1F4CC} "}
              {t("messages.pin.header_button", { count: pinnedCount })}
            </button>
          )}
          <button
            onClick={() => setStarredDrawerOpen(o => !o)}
            className="text-[10px] uppercase tracking-wider text-soft-grey hover:text-cyber-cyan border border-dim-green/40 hover:border-cyber-cyan/60 px-2 py-0.5 rounded transition-colors"
            title={t("messages.star.drawer_title")}
          >
            {"\u{2B50} "}
            {t("messages.star.header_button", { count: starredCount })}
          </button>
        </div>
      </div>
      <div className="flex-1 flex overflow-hidden">
        <div
          ref={containerRef}
          className="flex-1 overflow-y-auto px-4 py-3 space-y-1"
        >
          {messages.length === 0 && (
            <div className="text-soft-grey italic text-xs">
              {t("message_stream.empty")}
            </div>
          )}
          {messages.map((m, i) => {
            const arrow =
              m.kind === "incoming" ? "◀" : m.kind === "outgoing" ? "▶" : "·";
            const arrowColor =
              m.kind === "incoming"
                ? "text-cyber-cyan"
                : m.kind === "outgoing"
                ? "text-neon-green"
                : "text-soft-grey";

            const tampered = m.sig_ok === false;
            const isUnbound =
              m.kind === "incoming" && m.label.startsWith("?");
            const isFile = m.row_kind === "file" && m.file_meta;
            const fileHashBad =
              isFile && m.file_meta?.sha256_ok === false;
            // Wave 8G — image branch test. We render the inline thumbnail
            // ONLY when the row has a non-falsy `saved_path` (incoming
            // side) OR we'd fall back to the generic 📎 link otherwise.
            const isImageRow =
              isFile &&
              m.file_meta &&
              isImageFile(m.file_meta.mime, m.file_meta.filename);

            const isLatest = i === messages.length - 1;
            const enterClass = isLatest
              ? m.kind === "incoming"
                ? "pc-row-in-incoming"
                : m.kind === "outgoing"
                ? "pc-row-in-outgoing"
                : ""
              : "";

            // Wave 8G — pinned tint + control-row availability flag.
            // We only show pin/star toolbar for non-system rows that
            // carry a stable `msg_id` (the backend keys mutations on
            // msg_id, so a row missing one — i.e. legacy persisted
            // pre-feature rows — has nothing to pin/star).
            const canMutateState = m.kind !== "system" && !!m.msg_id;
            const pinnedTint = m.pinned ? "ring-1 ring-neon-magenta/40 bg-neon-magenta/5 " : "";

            // ── Disappearing-messages countdown ───────────────────────────
            // Compute remaining seconds against the live now-tick. Rows past
            // their deadline render with "0s" briefly until the 60s purge
            // sweep on the backend removes them and the parent drops them
            // from React state via the `messages_purged` event.
            const expiresInSecs =
              m.expires_at !== undefined
                ? Math.max(0, m.expires_at - Math.floor(Date.now() / 1000))
                : null;

            // ── Reply-quote click target ──────────────────────────────────
            // When a row carries `reply_to`, clicking the quote block scrolls
            // to the original message via the same data-attribute trick the
            // search-jump uses.
            const replyTarget = m.reply_to?.in_reply_to_msg_id;

            // Group reactions for the pill row; cheap O(N) per render but
            // the per-row reaction count is always tiny (single-digit).
            const grouped = m.reactions ? groupReactions(m.reactions) : [];

            return (
              <div
                key={i}
                data-msg-idx={i}
                data-msg-id={m.msg_id ?? ""}
                className={
                  "group relative flex flex-col text-sm leading-snug font-mono px-1 py-0.5 rounded " +
                  (tampered || fileHashBad
                    ? "bg-red-900/30 border border-red-500/50 "
                    : "") +
                  pinnedTint +
                  enterClass
                }
                title={
                  tampered
                    ? t("message_stream.tampered_title")
                    : fileHashBad
                    ? t("message_stream.hash_mismatch_title")
                    : undefined
                }
              >
                {/* Reply-quote block — a slim magenta-tinted card above the
                    message body. Click scrolls to the quoted row using the
                    same data-msg-id lookup the search-jump path uses. */}
                {m.reply_to && (
                  <button
                    onClick={() => {
                      if (!replyTarget) return;
                      const root = containerRef.current;
                      if (!root) return;
                      const target = root.querySelector<HTMLElement>(
                        `[data-msg-id="${CSS.escape(replyTarget)}"]`,
                      );
                      if (!target) return;
                      target.scrollIntoView({
                        behavior: "smooth",
                        block: "center",
                      });
                      target.classList.remove("pc-search-pulse");
                      void target.offsetWidth;
                      target.classList.add("pc-search-pulse");
                      window.setTimeout(() => {
                        target.classList.remove("pc-search-pulse");
                      }, 1600);
                    }}
                    className="ml-[80px] mb-0.5 block max-w-[80%] text-left text-xs px-2 py-1 rounded bg-neon-magenta/10 border-l-2 border-neon-magenta hover:bg-neon-magenta/20 transition-colors"
                    title={t("messages.reply.quote_jump_title")}
                  >
                    <span className="text-neon-magenta">{"↪"}</span>{" "}
                    <span className="text-soft-grey italic">
                      {m.reply_to.quoted_preview}
                    </span>
                  </button>
                )}

                <div className="flex items-start gap-3">
                <span className="text-soft-grey text-xs w-[64px] shrink-0">
                  {m.ts}
                  {expiresInSecs !== null && (
                    <span
                      className="block text-[9px] text-cyber-cyan/80"
                      title={t("messages.disappearing.row_title")}
                    >
                      {t("messages.disappearing.row_label", {
                        remaining: humanTtl(expiresInSecs),
                      })}
                    </span>
                  )}
                </span>
                <span className={`${arrowColor} font-bold w-4 shrink-0`}>
                  {arrow}
                </span>

                {(tampered || fileHashBad) && (
                  <span
                    className="text-red-400 font-bold w-4 shrink-0"
                    aria-label={
                      tampered
                        ? t("message_stream.warn_tampered")
                        : t("message_stream.warn_hash")
                    }
                  >
                    ⚠
                  </span>
                )}

                {/* Pinned indicator — small 📌 in front of the label so it
                    sits on the same baseline as other row glyphs. Hidden
                    when the row's hover-toolbar is opening anyway. */}
                {m.pinned && (
                  <span
                    className="text-neon-magenta text-xs shrink-0"
                    aria-label="pinned"
                    title={t("messages.pin.pinned_badge_title")}
                  >
                    {"\u{1F4CC}"}
                  </span>
                )}

                <span
                  className={
                    "w-[100px] shrink-0 truncate whitespace-pre " +
                    (tampered || fileHashBad
                      ? "text-red-300"
                      : "text-neon-magenta")
                  }
                >
                  {truncateLabel(m.label)}
                </span>

                {isImageRow && m.file_meta ? (
                  /* Wave 8G inline image branch. We render:
                     - sender-side echo (no saved_path): generic 📎 caption fallback,
                       since the bytes aren't on disk locally to convertFileSrc.
                     - receiver-side w/ saved_path + sha256_ok != false: <img>
                     - receiver-side w/ sha256_ok === false: red placeholder + ⚠
                       (per spec: NEVER render the actual image content for a
                       tampered file — security). */
                  fileHashBad ? (
                    <div className="flex flex-col gap-1">
                      <div className="w-[180px] h-[100px] bg-red-900/40 border border-red-500/60 rounded flex items-center justify-center text-red-300 text-xs uppercase tracking-wider">
                        ⚠ {t("message_stream.image_tampered_caption")}
                      </div>
                      <div className="text-[10px] text-red-300/80">
                        {m.file_meta.filename} ({humanSize(m.file_meta.size)})
                      </div>
                    </div>
                  ) : m.file_meta.saved_path ? (
                    <div className="flex flex-col gap-1">
                      <img
                        src={convertFileSrc(m.file_meta.saved_path)}
                        alt={m.file_meta.filename}
                        loading="lazy"
                        onClick={() =>
                          void openInDefaultViewer(m.file_meta!.saved_path!)
                        }
                        className="rounded border border-dim-green/40 hover:border-neon-magenta/60 transition-colors"
                        style={{
                          maxWidth: "320px",
                          maxHeight: "240px",
                          cursor: "pointer",
                        }}
                        title={t("message_stream.image_open_title", {
                          filename: m.file_meta.filename,
                        })}
                      />
                      <div className="text-[10px] text-soft-grey">
                        {t("message_stream.image_caption_received", {
                          filename: m.file_meta.filename,
                          size: humanSize(m.file_meta.size),
                        })}
                      </div>
                    </div>
                  ) : (
                    /* Outgoing-side echo — sender doesn't keep a local
                       copy, so we can't thumbnail. Fall through to the
                       generic 📎 caption + filename. */
                    <span className="flex items-center gap-2 text-neon-green/90">
                      <span aria-hidden="true">{"\u{1F4CE}"}</span>
                      <span className="text-soft-grey">
                        {t("message_stream.sent_label")}
                      </span>
                      <span className="font-bold">{m.file_meta.filename}</span>
                      <span className="text-soft-grey text-xs">
                        ({humanSize(m.file_meta.size)})
                      </span>
                    </span>
                  )
                ) : isFile && m.file_meta ? (
                  /* Non-image file row — original 📎 affordance. */
                  <span
                    className={
                      "flex items-center gap-2 " +
                      (fileHashBad
                        ? "text-red-200/90"
                        : m.kind === "outgoing"
                        ? "text-neon-green/90"
                        : "text-cyber-cyan")
                    }
                  >
                    <span aria-hidden="true">{"\u{1F4CE}"}</span>
                    <span className="text-soft-grey">
                      {m.kind === "outgoing"
                        ? t("message_stream.sent_label")
                        : t("message_stream.received_label")}
                    </span>
                    {onOpenFolder && m.file_meta.saved_path ? (
                      <button
                        onClick={onOpenFolder}
                        className="underline hover:text-neon-magenta hover:pc-brand-glow-magenta transition-colors"
                        title={t("message_stream.saved_to", { path: m.file_meta.saved_path })}
                      >
                        {m.file_meta.filename}
                      </button>
                    ) : (
                      <span className="font-bold">
                        {m.file_meta.filename}
                      </span>
                    )}
                    <span className="text-soft-grey text-xs">
                      ({humanSize(m.file_meta.size)})
                    </span>
                    {m.kind === "incoming" &&
                      onOpenFolder &&
                      m.file_meta.saved_path && (
                        <button
                          onClick={onOpenFolder}
                          className="ml-2 neon-button-magenta text-xs"
                          title={t("message_stream.open_folder_title")}
                        >
                          {t("message_stream.open_folder_button")}
                        </button>
                      )}
                  </span>
                ) : tampered ? (
                  /* Subtle dual-pseudo glitch — see .pc-glitch in styles.css.
                     data-text mirrors the body so ::before/::after can render
                     the offset cyan/magenta copies. */
                  <span
                    className="pc-glitch text-red-200/90"
                    data-text={m.body}
                  >
                    {m.body}
                  </span>
                ) : (
                  // Text body: pipe through markdown -> auto-linkify ->
                  // mention pills. Every stage HTML-escapes its inputs,
                  // so a literal <script> from a peer renders as visible
                  // text rather than executing. The system row inten-
                  // tionally bypasses the rich pipeline — its strings
                  // come from our own i18n bundle, never from a peer.
                  <RenderedBody
                    body={m.body}
                    isSystem={m.kind === "system"}
                    mentionLabels={mentionLabels}
                  />
                )}

                {isUnbound && onBindClick && (
                  <button
                    onClick={onBindClick}
                    className="ml-2 neon-button-magenta"
                    title={t("message_stream.bind_button_title", { label: m.label })}
                  >
                    {t("message_stream.bind_button")}
                  </button>
                )}

                {/* Wave 8G hover toolbar — Pin / Star buttons. Only visible
                    on hover (group-hover) and only for rows we can mutate
                    (i.e. with a stable msg_id). Sits absolute against the
                    row's right edge so it doesn't reflow message text. */}
                {canMutateState && (
                  <div className="absolute right-1 top-0 hidden group-hover:flex items-center gap-1 bg-bg-panel/95 border border-dim-green/40 rounded px-1 py-0.5 shadow">
                    <button
                      onClick={() => handleTogglePin(m.msg_id!, !!m.pinned)}
                      className={
                        "text-[11px] px-1 hover:text-neon-magenta transition-colors " +
                        (m.pinned ? "text-neon-magenta" : "text-soft-grey")
                      }
                      title={
                        m.pinned
                          ? t("messages.pin.unpin_title")
                          : t("messages.pin.pin_title")
                      }
                    >
                      {"\u{1F4CC}"}
                    </button>
                    <button
                      onClick={() => handleToggleStar(m.msg_id!, !!m.starred)}
                      className={
                        "text-[11px] px-1 hover:text-cyber-cyan transition-colors " +
                        (m.starred ? "text-cyber-cyan" : "text-soft-grey")
                      }
                      title={
                        m.starred
                          ? t("messages.star.unstar_title")
                          : t("messages.star.star_title")
                      }
                    >
                      {"\u{2B50}"}
                    </button>
                  </div>
                )}

                {/* Starred badge at the right edge — sits on the same row
                    as the delivery-state ticks, separated by a gap. */}
                {m.starred && (
                  <span
                    className="ml-auto text-cyber-cyan text-xs select-none"
                    aria-label="starred"
                    title={t("messages.star.starred_badge_title")}
                  >
                    {"\u{2B50}"}
                  </span>
                )}

                {/* Delivery-state ticks. `ml-auto` only kicks in when the
                    starred badge above isn't present (CSS auto-margin
                    collapse), so we add a manual `pl-2` for spacing. */}
                {m.kind === "outgoing" && m.delivery_state && (
                  <span
                    className={
                      (m.starred ? "" : "ml-auto") +
                      " pl-2 text-xs select-none " +
                      (m.delivery_state === "read"
                        ? "text-cyber-cyan font-bold"
                        : "text-soft-grey")
                    }
                    aria-label={`delivery: ${m.delivery_state}`}
                    title={`delivery: ${m.delivery_state}`}
                  >
                    {m.delivery_state === "sent" ? "✓" : "✓✓"}
                  </span>
                )}
                </div>

                {/* Reactions pill row — grouped by emoji, count + sender
                    tooltip. Click on an existing pill toggles the active
                    user's vote for that emoji (add if "you" hasn't reacted
                    with this emoji yet, remove otherwise). */}
                {grouped.length > 0 && (
                  <div className="ml-[180px] mt-0.5 flex flex-wrap gap-1">
                    {grouped.map(g => {
                      const youReacted = g.senders.some(s => s === "you");
                      return (
                        <button
                          key={g.emoji}
                          onClick={() => {
                            if (!m.msg_id || !onReact) return;
                            onReact(
                              m.msg_id,
                              g.emoji,
                              youReacted ? "remove" : "add",
                            );
                          }}
                          title={t("messages.reactions.pill_title", {
                            senders: g.senders.join(", "),
                          })}
                          className={
                            "text-xs px-1.5 py-0.5 rounded-full border transition-colors " +
                            (youReacted
                              ? "bg-neon-magenta/20 border-neon-magenta/60 text-neon-magenta"
                              : "bg-bg-elevated border-dim-green/40 text-soft-grey hover:border-neon-magenta/40")
                          }
                        >
                          <span aria-hidden="true">{g.emoji}</span>{" "}
                          <span className="font-bold">{g.count}</span>
                        </button>
                      );
                    })}
                  </div>
                )}

                {/* Hover toolbar — appears on row hover for incoming /
                    outgoing rows. System rows have no actionable content.
                    Sits absolute in the row's top-right so it doesn't
                    reflow the layout when toggling visibility. */}
                {m.kind !== "system" && (onReplyTo || onReact) && (
                  <div
                    className={
                      "absolute right-2 top-0 -translate-y-1/2 flex items-center gap-1 " +
                      "opacity-0 group-hover:opacity-100 transition-opacity " +
                      "bg-bg-elevated border border-dim-green/40 rounded shadow-md px-1 py-0.5"
                    }
                  >
                    {onReplyTo && m.msg_id && (
                      <button
                        onClick={() => {
                          if (!m.msg_id) return;
                          onReplyTo(m.msg_id, m.body);
                        }}
                        className="text-xs text-cyber-cyan hover:text-neon-magenta px-1 transition-colors"
                        title={t("messages.reply.toolbar_title")}
                      >
                        {t("messages.reply.toolbar_button")}
                      </button>
                    )}
                    {onReact && m.msg_id && (
                      <button
                        onClick={() => setPickerForIdx(i)}
                        className="text-xs text-cyber-cyan hover:text-neon-magenta px-1 transition-colors"
                        title={t("messages.reactions.toolbar_title")}
                      >
                        {t("messages.reactions.toolbar_button")}
                      </button>
                    )}
                  </div>
                )}

                {pickerForIdx === i && onReact && m.msg_id && (
                  <EmojiPicker
                    onSelect={emoji => {
                      if (!m.msg_id) return;
                      // If "you" already reacted with this exact emoji,
                      // toggle remove; otherwise add. The backend de-dupes
                      // identical add events server-side too, but we make
                      // the UI feel snappy by inferring intent here.
                      const youReactedSame = (m.reactions ?? []).some(
                        r => r.sender_label === "you" && r.emoji === emoji,
                      );
                      onReact(
                        m.msg_id,
                        emoji,
                        youReactedSame ? "remove" : "add",
                      );
                    }}
                    onClose={() => setPickerForIdx(null)}
                  />
                )}
              </div>
            );
          })}
          <div ref={bottomRef} />
        </div>

        {/* ── Pinned drawer (per-conversation) ───────────────────────── */}
        {pinnedDrawerOpen && (
          <aside className="w-[280px] shrink-0 border-l border-neon-magenta/40 bg-bg-panel/85 backdrop-blur-[1px] overflow-y-auto p-3 text-xs">
            <div className="flex items-center justify-between mb-2">
              <span className="text-neon-magenta uppercase tracking-widest text-[10px] font-display">
                {"\u{1F4CC} "}
                {t("messages.pin.drawer_title")}
              </span>
              <button
                onClick={() => setPinnedDrawerOpen(false)}
                className="text-soft-grey hover:text-neon-green text-xs px-1"
              >
                ✕
              </button>
            </div>
            {pinnedDrawerRows.length === 0 ? (
              <div className="text-soft-grey italic">
                {t("messages.pin.drawer_empty")}
              </div>
            ) : (
              <ul className="space-y-2">
                {pinnedDrawerRows.map((m, i) => (
                  <li
                    key={i}
                    onClick={() => m.msg_id && jumpToMsgId(m.msg_id)}
                    className="cursor-pointer border border-dim-green/40 hover:border-neon-magenta/60 rounded p-2 transition-colors"
                    title={t("messages.pin.jump_title")}
                  >
                    <div className="text-[10px] text-soft-grey">
                      {m.timestamp} · {m.sender_label}
                    </div>
                    <div className="text-neon-green truncate">
                      {m.plaintext}
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </aside>
        )}

        {/* ── Starred drawer (cross-conversation) ────────────────────── */}
        {starredDrawerOpen && (
          <aside className="w-[280px] shrink-0 border-l border-cyber-cyan/40 bg-bg-panel/85 backdrop-blur-[1px] overflow-y-auto p-3 text-xs">
            <div className="flex items-center justify-between mb-2">
              <span className="text-cyber-cyan uppercase tracking-widest text-[10px] font-display">
                {"\u{2B50} "}
                {t("messages.star.drawer_title")}
              </span>
              <button
                onClick={() => setStarredDrawerOpen(false)}
                className="text-soft-grey hover:text-neon-green text-xs px-1"
              >
                ✕
              </button>
            </div>
            {starredDrawerRows.length === 0 ? (
              <div className="text-soft-grey italic">
                {t("messages.star.drawer_empty")}
              </div>
            ) : (
              <ul className="space-y-2">
                {starredDrawerRows.map((m, i) => (
                  <li
                    key={i}
                    onClick={() => {
                      // Switch conversation first so the row is in the
                      // active stream, then jump on the next tick once
                      // App.tsx has re-rendered with the new active
                      // contact.
                      if (
                        onSwitchConversation &&
                        m.sender_label &&
                        m.sender_label !== activeLabel
                      ) {
                        onSwitchConversation(m.sender_label);
                      }
                      if (m.msg_id) {
                        const id = m.msg_id;
                        window.setTimeout(() => jumpToMsgId(id), 50);
                      }
                    }}
                    className="cursor-pointer border border-dim-green/40 hover:border-cyber-cyan/60 rounded p-2 transition-colors"
                    title={t("messages.star.jump_title")}
                  >
                    <div className="text-[10px] text-soft-grey">
                      {m.timestamp} · {m.sender_label}
                    </div>
                    <div className="text-neon-green truncate">
                      {m.plaintext}
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </aside>
        )}
      </div>
    </section>
  );
}

/// Memoised text-body renderer. Runs the markdown → auto-link → mention
/// pipeline once per (body, mentionLabels) tuple, then plugs the result
/// in via `dangerouslySetInnerHTML`. The pipeline escapes every text run
/// before composing HTML, so a peer's literal `<script>alert(1)</script>`
/// shows up as visible text (not executed).
function RenderedBody({
  body,
  isSystem,
  mentionLabels,
}: {
  body: string;
  isSystem: boolean;
  mentionLabels: string[];
}) {
  const html = useMemo(() => {
    if (isSystem) return null;
    const md = renderMarkdown(body);
    const linked = autoLinkify(md);
    const { html: withMentions } = detectMentions(linked, mentionLabels);
    return withMentions;
    // mentionLabels is memoised by the parent; re-run only when the
    // body text or label-set actually changes.
  }, [body, isSystem, mentionLabels]);

  if (isSystem) {
    return <span className="text-soft-grey italic">{body}</span>;
  }
  return (
    <span
      className="text-neon-green/90 break-words [&_a]:break-all"
      // eslint-disable-next-line react/no-danger
      dangerouslySetInnerHTML={{ __html: html ?? "" }}
    />
  );
}
