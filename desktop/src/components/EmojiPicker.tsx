import { useEffect, useRef } from "react";

interface Props {
  /// Called when the user clicks an emoji. Parent decides whether this
  /// represents an "add" or "remove" action against the target message.
  onSelect: (emoji: string) => void;
  /// Dismiss the popover (click-outside, Escape, or after a selection).
  onClose: () => void;
}

/// Inline 24-emoji palette. We deliberately don't pull in a heavier emoji
/// library for an MVP — the most common reactions cover ~95% of usage.
/// Layout is a 6-wide grid so the popover stays compact above the
/// triggering message row.
const EMOJIS = [
  "\u{1F44D}", // 👍
  "\u{2764}\u{FE0F}", // ❤️
  "\u{1F602}", // 😂
  "\u{1F525}", // 🔥
  "\u{1F389}", // 🎉
  "\u{1F440}", // 👀
  "\u{2705}", // ✅
  "\u{274C}", // ❌
  "\u{1F4AF}", // 💯
  "\u{1F680}", // 🚀
  "\u{1F914}", // 🤔
  "\u{1F622}", // 😢
  "\u{1F64F}", // 🙏
  "\u{1F44F}", // 👏
  "\u{26A1}", // ⚡
  "\u{1F31F}", // 🌟
  "\u{1F44E}", // 👎
  "\u{1F60D}", // 😍
  "\u{1F923}", // 🤣
  "\u{1F61E}", // 😞
  "\u{1F92F}", // 🤯
  "\u{1F4A1}", // 💡
  "\u{1F4A5}", // 💥
  "\u{1F47B}", // 👻
];

export default function EmojiPicker({ onSelect, onClose }: Props) {
  const rootRef = useRef<HTMLDivElement>(null);

  // Click-outside + Escape close. We attach to `document` so a click
  // anywhere else (incl. another emoji popover on a different row) tears
  // this one down cleanly without leaving stale popovers.
  useEffect(() => {
    function onDocClick(e: MouseEvent) {
      if (!rootRef.current) return;
      if (rootRef.current.contains(e.target as Node)) return;
      onClose();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    }
    // Defer the click handler by one tick so the very click that opened
    // the popover doesn't immediately close it.
    const t = window.setTimeout(() => {
      document.addEventListener("mousedown", onDocClick);
    }, 0);
    document.addEventListener("keydown", onKey);
    return () => {
      window.clearTimeout(t);
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  return (
    <div
      ref={rootRef}
      className="absolute z-40 mt-1 grid grid-cols-6 gap-1 p-2 bg-bg-elevated border border-neon-magenta/60 rounded-md shadow-lg"
      role="dialog"
      aria-label="emoji picker"
    >
      {EMOJIS.map(e => (
        <button
          key={e}
          onClick={() => {
            onSelect(e);
            onClose();
          }}
          className="text-lg leading-none px-1.5 py-1 rounded hover:bg-neon-magenta/20 transition-colors"
          title={e}
        >
          {e}
        </button>
      ))}
    </div>
  );
}
