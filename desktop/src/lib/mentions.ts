// `@<label>` mention rewriter. Runs as the LAST stage of the chat-body
// rendering pipeline (after markdown + auto-link). Walks the HTML in
// text-context only — never rewrites inside an `<a>` (would clobber a
// URL that happens to contain `@`) and never inside an attribute value.
//
// A "known label" is any label currently visible to the user: their own
// `me.json` label, every member of the active MLS group's directory,
// every 1:1 contact label. The caller is responsible for assembling the
// list — `App.tsx` passes
//   [me.label, ...mlsDirectory.map(m => m.label), ...contacts.map(c => c.label)]
//
// Local-mention vs. peer-mention is decided downstream (in App.tsx's
// `mls_message` listener) — this module just rewrites the HTML and
// reports back which labels were mentioned so the caller can fire the
// appropriate notification.

import { escapeHtml } from "./markdown";

/// Characters that are valid inside a mention label. We accept the same
/// alphabet that the wizard / contact-add modal accepts: ASCII letters,
/// digits, underscore, dot, hyphen. Stops at whitespace, punctuation,
/// HTML tag boundaries, etc.
const MENTION_CHAR = /[A-Za-z0-9_.-]/;

export interface MentionResult {
  /// HTML with every `@<known-label>` substring replaced with a magenta
  /// pill <span>. Untouched if no known labels appear.
  html: string;
  /// Distinct list of labels that were actually mentioned (deduped,
  /// case-preserved as written by the sender — but matched
  /// case-insensitively against `knownLabels`).
  mentions: string[];
}

/// Rewrite `@label` runs in `html` to the magenta-pill HTML, but only
/// where `label` matches one of `knownLabels` (case-insensitive). Runs
/// only in text-context — never inside `<a>` content or attributes.
export function detectMentions(
  html: string,
  knownLabels: string[],
): MentionResult {
  if (!html || knownLabels.length === 0) {
    return { html, mentions: [] };
  }
  // Build a case-folded set for O(1) lookup, then a parallel map back to
  // the canonical label so the rendered pill shows the user's preferred
  // casing rather than whatever the sender typed.
  const knownByLower = new Map<string, string>();
  for (const lbl of knownLabels) {
    if (!lbl) continue;
    knownByLower.set(lbl.toLowerCase(), lbl);
  }

  let out = "";
  let i = 0;
  let inTag = false;
  let inAnchor = 0;
  const mentionedSet = new Set<string>();

  while (i < html.length) {
    const c = html[i];
    if (!inTag && c === "<") {
      out += c;
      inTag = true;
      // Track <a> depth so we skip @ inside link text.
      const lower = html.slice(i, i + 4).toLowerCase();
      if (lower.startsWith("<a") && /[\s>]/.test(html[i + 2] || "")) {
        inAnchor += 1;
      } else if (lower === "</a>") {
        inAnchor = Math.max(0, inAnchor - 1);
      }
      i += 1;
      continue;
    }
    if (inTag) {
      out += c;
      if (c === ">") inTag = false;
      i += 1;
      continue;
    }
    if (inAnchor > 0) {
      out += c;
      i += 1;
      continue;
    }
    // Text context: look for `@` at a word boundary.
    if (c === "@") {
      const prev = i > 0 ? html[i - 1] : "";
      const atWordBoundary =
        i === 0 ||
        /\s/.test(prev) ||
        prev === ">" ||
        /[(\[{,;:!?"']/.test(prev);
      if (atWordBoundary) {
        // Scan forward for label characters.
        let j = i + 1;
        while (j < html.length && MENTION_CHAR.test(html[j])) {
          j += 1;
        }
        if (j > i + 1) {
          const label = html.slice(i + 1, j);
          const canonical = knownByLower.get(label.toLowerCase());
          if (canonical) {
            mentionedSet.add(canonical);
            out += `<span class="bg-neon-magenta/20 text-neon-magenta px-1 rounded font-bold">@${escapeHtml(canonical)}</span>`;
            i = j;
            continue;
          }
        }
      }
    }
    out += c;
    i += 1;
  }

  return { html: out, mentions: Array.from(mentionedSet) };
}

/// Plaintext (NOT HTML) check — used by the `mls_message` listener to
/// decide whether to fire the loud "you were mentioned" notification.
/// Matches the same word-boundary + label-char rules as `detectMentions`.
export function plaintextMentions(
  text: string,
  knownLabels: string[],
): string[] {
  if (!text || knownLabels.length === 0) return [];
  const knownByLower = new Map<string, string>();
  for (const lbl of knownLabels) {
    if (!lbl) continue;
    knownByLower.set(lbl.toLowerCase(), lbl);
  }
  const found = new Set<string>();
  let i = 0;
  while (i < text.length) {
    if (text[i] === "@") {
      const prev = i > 0 ? text[i - 1] : "";
      const atWordBoundary =
        i === 0 ||
        /\s/.test(prev) ||
        /[(\[{,;:!?"']/.test(prev);
      if (atWordBoundary) {
        let j = i + 1;
        while (j < text.length && MENTION_CHAR.test(text[j])) j += 1;
        if (j > i + 1) {
          const lbl = text.slice(i + 1, j).toLowerCase();
          const canonical = knownByLower.get(lbl);
          if (canonical) found.add(canonical);
          i = j;
          continue;
        }
      }
    }
    i += 1;
  }
  return Array.from(found);
}
