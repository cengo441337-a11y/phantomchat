import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Contact, SearchHit } from "../types";

interface Props {
  contacts: Contact[];
  /// Closes the panel without jumping. Wired to the X button + Escape key.
  onClose: () => void;
  /// User picked a result row. Receives the index into the on-disk
  /// `messages.json` array (== position in the React MessageStream's
  /// `messages` prop). Parent should scroll-to-row + pulse-highlight.
  onJumpTo: (msgIdx: number) => void;
}

const DEBOUNCE_MS = 200;
const MAX_RESULTS = 100;

/// Compact search panel — slides down from the top of the chat area.
/// Debounced text input + sender-filter dropdown invoke the backend
/// `search_messages` command on input change, then render a scrollable
/// result list. Each result shows `[ts]` `[sender]` `body…` with the
/// matching substrings highlighted in magenta. Clicking a row triggers
/// `onJumpTo` so the parent can scroll the main MessageStream to that
/// row and play the search-pulse animation.
export default function SearchPanel({ contacts, onClose, onJumpTo }: Props) {
  const [query, setQuery] = useState("");
  const [senderFilter, setSenderFilter] = useState<string>("");
  const [results, setResults] = useState<SearchHit[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Bump on every input change so an in-flight search whose result
  // arrives AFTER a newer one can be discarded by comparing tokens.
  const reqToken = useRef(0);

  // Auto-focus the input on mount so the user can start typing
  // immediately after Ctrl+F.
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Escape closes the panel — convention for any "command palette"
  // style overlay. We listen on window because the input might or
  // might not have focus depending on user interaction.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Debounced search effect. Re-runs on every query / sender-filter
  // change but coalesces bursts so we don't hammer the backend on
  // every keystroke. Empty query → clear results without round-tripping.
  useEffect(() => {
    if (debounceTimer.current) clearTimeout(debounceTimer.current);
    const trimmed = query.trim();
    if (trimmed.length === 0) {
      setResults([]);
      setError(null);
      setIsLoading(false);
      return;
    }
    setIsLoading(true);
    debounceTimer.current = setTimeout(async () => {
      const myToken = ++reqToken.current;
      try {
        const hits = await invoke<SearchHit[]>("search_messages", {
          query: trimmed,
          senderFilter: senderFilter.length > 0 ? senderFilter : null,
          limit: MAX_RESULTS,
        });
        // Drop stale responses — the user typed again before this
        // returned. Without this we'd flicker old results back in.
        if (myToken !== reqToken.current) return;
        setResults(hits);
        setError(null);
      } catch (e) {
        if (myToken !== reqToken.current) return;
        setError(String(e));
        setResults([]);
      } finally {
        if (myToken === reqToken.current) setIsLoading(false);
      }
    }, DEBOUNCE_MS);
    return () => {
      if (debounceTimer.current) clearTimeout(debounceTimer.current);
    };
  }, [query, senderFilter]);

  // De-duplicated sender labels for the dropdown — we only show
  // contacts the user actually has bound, plus a "(any)" option.
  const senderOptions = useMemo(() => {
    const seen = new Set<string>();
    const out: string[] = [];
    for (const c of contacts) {
      if (!seen.has(c.label)) {
        seen.add(c.label);
        out.push(c.label);
      }
    }
    return out;
  }, [contacts]);

  return (
    <div className="border-b border-neon-magenta/40 bg-bg-panel/95 backdrop-blur-sm shadow-neon-magenta">
      {/* ── Search bar row ─────────────────────────────────────────── */}
      <div className="flex items-center gap-2 px-3 py-2">
        <span className="text-neon-magenta text-xs font-display uppercase tracking-widest">
          search
        </span>
        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={e => setQuery(e.target.value)}
          placeholder="type to search messages…"
          className="neon-input flex-1 text-sm py-1"
          aria-label="Search messages"
        />
        <select
          value={senderFilter}
          onChange={e => setSenderFilter(e.target.value)}
          className="neon-input text-xs py-1 max-w-[160px]"
          aria-label="Filter by sender"
        >
          <option value="">(any sender)</option>
          {senderOptions.map(lbl => (
            <option key={lbl} value={lbl}>
              {lbl}
            </option>
          ))}
        </select>
        <button
          onClick={onClose}
          className="neon-button-magenta"
          title="Close (Esc)"
          aria-label="Close search"
        >
          ×
        </button>
      </div>

      {/* ── Results list ───────────────────────────────────────────── */}
      <div className="max-h-[40vh] overflow-y-auto border-t border-dim-green/30">
        {error && (
          <div className="px-3 py-2 text-xs text-red-300">
            search error: {error}
          </div>
        )}
        {!error && query.trim().length === 0 && (
          <div className="px-3 py-2 text-xs text-soft-grey italic">
            (start typing — search runs after 200ms of idle)
          </div>
        )}
        {!error && query.trim().length > 0 && results.length === 0 && !isLoading && (
          <div className="px-3 py-2 text-xs text-soft-grey italic">
            no matches
          </div>
        )}
        {isLoading && results.length === 0 && (
          <div className="px-3 py-2 text-xs text-cyber-cyan italic">
            searching…
          </div>
        )}
        {results.map((hit, i) => (
          <SearchResultRow
            key={`${hit.msg_idx}-${i}`}
            hit={hit}
            onClick={() => onJumpTo(hit.msg_idx)}
          />
        ))}
        {results.length >= MAX_RESULTS && (
          <div className="px-3 py-2 text-xs text-soft-grey italic">
            showing first {MAX_RESULTS} matches — refine the query for more
          </div>
        )}
      </div>
    </div>
  );
}

interface RowProps {
  hit: SearchHit;
  onClick: () => void;
}

/// One result row. Renders `[ts] [sender] body` with the body's match
/// ranges sliced into highlighted spans. We splice on byte offsets
/// because the backend computes ranges that way; for ASCII queries this
/// is byte- and char-equivalent. Non-ASCII inside the body still
/// renders correctly because we're slicing a JS string at indices —
/// React DOM doesn't care that we're not on a code-point boundary as
/// long as the slice is consistent (the backend's case-folding only
/// affects ASCII so this is a safe simplification for an MVP).
function SearchResultRow({ hit, onClick }: RowProps) {
  const segments = useMemo(() => sliceWithHighlights(hit.plaintext, hit.match_ranges), [hit]);
  const isFileRow = hit.kind === "file";
  return (
    <button
      type="button"
      onClick={onClick}
      className="block w-full text-left px-3 py-1.5 border-b border-dim-green/20 hover:bg-neon-magenta/10 transition-colors font-mono text-sm"
    >
      <span className="text-soft-grey text-xs mr-2">[{hit.timestamp}]</span>
      <span className="text-neon-magenta mr-2">{hit.sender_label}</span>
      {isFileRow && hit.match_ranges.length === 0 ? (
        // Filename-only match — no per-char highlights to render. Tag
        // the row visually so the user knows what fired the hit.
        <span className="text-cyber-cyan italic">[file] {hit.plaintext}</span>
      ) : (
        <span className="text-neon-green/90">{segments}</span>
      )}
    </button>
  );
}

/// Splice `text` into a list of highlighted / non-highlighted spans
/// using the backend's `[start, end)` byte-offset pairs. Ranges are
/// assumed to be sorted ascending and non-overlapping (which is what
/// `find_match_ranges` produces — overlap-only-by-1-step search isn't
/// truly overlapping at the highlight level since spans are >0 length).
/// Defensive: if a range falls outside the string we skip it instead
/// of throwing, so a stale render never crashes the panel.
function sliceWithHighlights(
  text: string,
  ranges: Array<[number, number]>,
): JSX.Element[] {
  if (ranges.length === 0) {
    return [<span key="all">{text}</span>];
  }
  // Coalesce overlapping/touching ranges so the React keys stay simple
  // and the rendered DOM doesn't double-wrap a single highlight span.
  const sorted = [...ranges].sort((a, b) => a[0] - b[0]);
  const merged: Array<[number, number]> = [];
  for (const [s, e] of sorted) {
    const last = merged[merged.length - 1];
    if (last && s <= last[1]) {
      last[1] = Math.max(last[1], e);
    } else {
      merged.push([s, e]);
    }
  }

  const out: JSX.Element[] = [];
  let cursor = 0;
  let key = 0;
  for (const [start, end] of merged) {
    if (start < 0 || end > text.length || start >= end) continue;
    if (start > cursor) {
      out.push(<span key={`p${key++}`}>{text.slice(cursor, start)}</span>);
    }
    out.push(
      <span
        key={`h${key++}`}
        className="bg-neon-magenta/30 text-white rounded-sm px-0.5"
      >
        {text.slice(start, end)}
      </span>,
    );
    cursor = end;
  }
  if (cursor < text.length) {
    out.push(<span key={`p${key++}`}>{text.slice(cursor)}</span>);
  }
  return out;
}
