// Floating mention auto-complete dropdown shown above the InputBar when
// the user types `@` at a word-boundary inside the input. The popover is
// positioned with a simple bottom-anchored absolute layout so it grows
// upwards from the input — chat windows are always tall and we don't
// want the popover to spill below the visible area.
//
// Keyboard handling lives in the parent (`InputBar.tsx`) because the
// arrow / Enter / Tab / Escape keys need to interact with the input
// caret position. This component only renders + handles mouse clicks.

import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { MlsMemberRef } from "../types";

interface Props {
  /// Filtered list of MLS group members matching the current `@<prefix>`.
  /// The parent does the filtering so this component stays pure UI.
  candidates: MlsMemberRef[];
  /// Index of the currently-highlighted candidate (driven by ↑/↓ in the
  /// parent). Click handlers also dispatch a `select` for the row that
  /// was clicked, regardless of the keyboard-highlight state.
  activeIdx: number;
  /// Called when the user picks a candidate (Enter / Tab / mouse-click).
  /// The parent rewrites the input value so the partial `@<prefix>`
  /// becomes `@<full-label> ` (trailing space convenience).
  onSelect: (label: string) => void;
  /// Called when the user clicks outside the popover. Parent dismisses.
  onDismiss: () => void;
}

export default function MentionPopover({
  candidates,
  activeIdx,
  onSelect,
  onDismiss,
}: Props) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);

  // Click-outside dismiss. We attach to `mousedown` so the close fires
  // before the input's own `blur` handler clobbers the popover state.
  useEffect(() => {
    const onDocMouseDown = (e: MouseEvent) => {
      const root = containerRef.current;
      if (!root) return;
      if (root.contains(e.target as Node)) return;
      onDismiss();
    };
    document.addEventListener("mousedown", onDocMouseDown);
    return () => document.removeEventListener("mousedown", onDocMouseDown);
  }, [onDismiss]);

  // Auto-scroll the highlighted row into view when arrow-keys drive past
  // the visible window. `scrollIntoView` with `block: "nearest"` keeps
  // the popover layout stable.
  useEffect(() => {
    const root = containerRef.current;
    if (!root) return;
    const row = root.querySelector<HTMLElement>(
      `[data-mention-idx="${activeIdx}"]`,
    );
    row?.scrollIntoView({ block: "nearest" });
  }, [activeIdx]);

  if (candidates.length === 0) {
    return (
      <div
        ref={containerRef}
        className="absolute bottom-full left-0 mb-1 z-40 bg-bg-elevated border border-dim-green/60 rounded-md shadow-neon-magenta px-3 py-2 text-xs text-soft-grey italic"
      >
        {t("mention.empty")}
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      className="absolute bottom-full left-0 mb-1 z-40 bg-bg-elevated border border-dim-green/60 rounded-md shadow-neon-magenta min-w-[220px] max-h-48 overflow-y-auto"
      role="listbox"
      aria-label={t("mention.aria_label")}
    >
      <div className="px-2 py-1 text-[10px] uppercase tracking-widest text-soft-grey border-b border-dim-green/40">
        {t("mention.header")}
      </div>
      {candidates.map((c, i) => {
        const isActive = i === activeIdx;
        return (
          <button
            key={c.signing_pub_hex}
            data-mention-idx={i}
            type="button"
            // Use `mousedown` so the input doesn't lose focus before the
            // selection fires (which would close the popover and drop
            // the click).
            onMouseDown={e => {
              e.preventDefault();
              onSelect(c.label);
            }}
            className={
              "w-full text-left px-2 py-1 text-xs flex items-center justify-between gap-2 transition-colors " +
              (isActive
                ? "bg-neon-magenta/20 text-neon-magenta"
                : "text-neon-green hover:bg-bg-panel hover:text-cyber-cyan")
            }
            role="option"
            aria-selected={isActive}
          >
            <span className="font-bold">@{c.label}</span>
            <span
              className="text-[10px] text-soft-grey font-mono truncate ml-2"
              title={c.signing_pub_hex}
            >
              {c.signing_pub_hex.slice(0, 8)}…
            </span>
          </button>
        );
      })}
    </div>
  );
}
