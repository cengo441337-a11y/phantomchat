import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { MsgLine } from "../types";

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

export default function MessageStream({
  messages,
  activeLabel,
  onBindClick,
  onOpenFolder,
  highlightedIdx,
  onMarkRead,
}: Props) {
  const { t } = useTranslation();
  const bottomRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
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

          return (
            <div
              key={i}
              data-msg-idx={i}
              className={
                "flex items-start gap-3 text-sm leading-snug font-mono px-1 py-0.5 rounded " +
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
              <span className="text-soft-grey text-xs w-[64px] shrink-0">
                {m.ts}
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
                <span
                  className={
                    m.kind === "system"
                      ? "text-soft-grey italic"
                      : "text-neon-green/90"
                  }
                >
                  {m.body}
                </span>
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
          );
        })}
        <div ref={bottomRef} />
      </div>
    </section>
  );
}
