import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { MsgLine } from "../types";
import { renderMarkdown } from "../lib/markdown";
import { autoLinkify } from "../lib/linkify";
import { detectMentions } from "../lib/mentions";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { MsgLine, ReactionEntry } from "../types";
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

  // ── IntersectionObserver: fire `mark_read` for incoming rows ──────────
  // We mount a single observer at the container scope and re-observe rows
  // whenever the messages array grows. Triggers only when:
  //   1. The window is focused (no read receipts while backgrounded — that
  //      would lie to the sender about whether we actually saw the message).
  //   2. The row is at least 60% visible (avoids edge-of-viewport false
  //      positives on inertial scroll).
  //   3. The row is incoming with a non-empty `msg_id` and a real contact
  //      label (skip "INBOX" / "INBOX!" / "?<hex>" — no contact to ack).
  //   4. We haven't already fired for this `msg_id`.
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
          // Skip unbound / unattributed senders — backend would 404 since
          // there's no resolvable contact label to ship the receipt to.
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
  // Skip when a search-jump is in flight (highlightedIdx set) so the
  // jump's `scrollIntoView` isn't immediately undone by the bottom-pin.
  useEffect(() => {
    if (highlightedIdx !== undefined) return;
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, highlightedIdx]);

  // Search-panel "jump to result" target. Locate the row by data-attribute
  // (more robust than refs since the row count is dynamic), scroll it
  // into view, then add the pulse class. The class is removed after the
  // animation duration so a second jump to the same idx still fires.
  useEffect(() => {
    if (highlightedIdx === undefined) return;
    const root = containerRef.current;
    if (!root) return;
    const row = root.querySelector<HTMLElement>(
      `[data-msg-idx="${highlightedIdx}"]`,
    );
    if (!row) return;
    row.scrollIntoView({ behavior: "smooth", block: "center" });
    // Re-trigger the animation by removing/re-adding the class. We
    // also strip it again after 1500ms so the row stays interactive
    // for a future jump (CSS animations only re-fire on class toggle).
    row.classList.remove("pc-search-pulse");
    // Force a reflow so the browser observes the class removal before
    // we re-add it — without this the animation wouldn't restart.
    void row.offsetWidth;
    row.classList.add("pc-search-pulse");
    const t = window.setTimeout(() => {
      row.classList.remove("pc-search-pulse");
    }, 1600);
    return () => window.clearTimeout(t);
  }, [highlightedIdx]);

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
      <div className="px-4 py-1.5 text-xs text-cyber-cyan uppercase tracking-widest border-b border-dim-green/30 font-display">
        {title}
      </div>
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

          // sig_ok defaults to `true` (system/outgoing have no signature
          // semantics; old persisted rows pre-feature also default true).
          const tampered = m.sig_ok === false;
          const isUnbound =
            m.kind === "incoming" && m.label.startsWith("?");
          const isFile = m.row_kind === "file" && m.file_meta;
          // File-row: hash mismatch flips us into a hard-warn red state
          // distinct from the sig-tampered tint so the user can tell the
          // two failure modes apart.
          const fileHashBad =
            isFile && m.file_meta?.sha256_ok === false;

          // Run-once enter animation. We only animate the most recent
          // batch — older rows would re-trigger on every re-render
          // otherwise, which would look chaotic. Cheapest approach:
          // animate only the row at the very bottom (`i === messages.length - 1`).
          // The CSS `animation: ... both` keeps the final frame stable for
          // any row that was previously animated.
          const isLatest = i === messages.length - 1;
          const enterClass = isLatest
            ? m.kind === "incoming"
              ? "pc-row-in-incoming"
              : m.kind === "outgoing"
              ? "pc-row-in-outgoing"
              : ""
            : "";

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
                "group relative px-1 py-0.5 rounded " +
                (tampered || fileHashBad
                  ? "bg-red-900/30 border border-red-500/50 "
                  : "") +
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

              <div className="flex items-start gap-3 text-sm leading-snug font-mono">
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

              {isFile && m.file_meta ? (
                /* File row: 📎 + filename (link-style) + (size) in dim grey.
                   For received rows (kind === "incoming") with a saved_path,
                   the filename click triggers `onOpenFolder` so the user lands
                   in the directory containing the file. Sender-side rows
                   ("outgoing") just render statically. */
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
                  {/* "Open folder" pill on incoming rows — reachable even
                      after the filename click, in case the user clicked
                      somewhere unhelpful. Hidden for outgoing rows since the
                      sender has nothing local to open. */}
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

              {/* Delivery-state ticks on outgoing rows. Pushed to the
                  right edge with `ml-auto` so they sit flush against the
                  pane border regardless of body length. Three tiers:
                    "sent"      → single grey ✓
                    "delivered" → double grey ✓✓
                    "read"      → double cyan ✓✓ (cyber-cyan + bold) */}
              {m.kind === "outgoing" && m.delivery_state && (
                <span
                  className={
                    "ml-auto pl-2 text-xs select-none " +
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
