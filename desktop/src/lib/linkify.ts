// Bare-URL auto-linker. Runs AFTER `renderMarkdown` so we don't double-
// link `[text](https://x)`-style markdown links — those are already
// wrapped in `<a>` tags by the markdown renderer, and the regex below
// is anchored on a word boundary so it skips URLs that are already
// inside an attribute (`href="..."`).
//
// We can't use a naive `replace` over the WHOLE HTML because that would
// re-process URLs sitting inside an attribute value. Instead we walk the
// HTML in a state machine, only rewriting URL-shaped runs that appear
// in TEXT context (between tags), never inside the angle-brackets of a
// tag itself.

import { escapeHtml } from "./markdown";

/// `https?://` followed by URL-safe characters. Stops at whitespace,
/// `<` (next tag), `"` / `'` (would be inside an attribute — we don't
/// match in text-context so this is belt-and-braces), and a few sentence-
/// punctuation chars that are ambiguous as terminators (`.`, `,`, `;`).
const URL_RE = /\bhttps?:\/\/[^\s<>"'`]+/g;

/// Trailing punctuation we strip from a matched URL — sentence enders
/// that aren't actually part of the link. Re-appended after the `</a>`.
const TRAILING_PUNCT = /[.,;!?)]+$/;

/// Apply auto-link rewriting to a chunk of HTML. URLs already wrapped
/// in `<a>` are left alone (because we only scan text-context runs);
/// bare URLs become `<a href=... target=_blank rel=noopener>` anchors.
export function autoLinkify(html: string): string {
  let out = "";
  let i = 0;
  let inTag = false;
  let inAnchor = 0; // depth of nested <a> — skip auto-linking inside.
  let chunkStart = 0;

  // Helper: flush text from `chunkStart` to `end` with URL-rewriting.
  const flushText = (end: number) => {
    if (end <= chunkStart) return;
    const chunk = html.slice(chunkStart, end);
    if (inAnchor > 0) {
      out += chunk;
    } else {
      out += chunk.replace(URL_RE, m => {
        // Strip trailing sentence punctuation and re-append it after
        // closing the anchor — looks more natural inline.
        let url = m;
        let trail = "";
        const trailMatch = url.match(TRAILING_PUNCT);
        if (trailMatch) {
          trail = trailMatch[0];
          url = url.slice(0, url.length - trail.length);
        }
        if (!url) return m;
        return `<a href="${escapeHtml(url)}" target="_blank" rel="noopener noreferrer" class="text-cyber-cyan underline hover:text-neon-green">${escapeHtml(url)}</a>${trail}`;
      });
    }
  };

  while (i < html.length) {
    const c = html[i];
    if (!inTag && c === "<") {
      // Flush the text segment that just ended, then enter tag mode.
      flushText(i);
      inTag = true;
      // Detect <a ...> open / </a> close so we don't double-link inside.
      // Lower-case match to be robust against renderers that emit `<A>`.
      if (html.slice(i, i + 2).toLowerCase() === "<a" && /[\s>]/.test(html[i + 2] || "")) {
        inAnchor += 1;
      } else if (html.slice(i, i + 4).toLowerCase() === "</a>") {
        inAnchor = Math.max(0, inAnchor - 1);
      }
      chunkStart = i;
      i += 1;
      continue;
    }
    if (inTag && c === ">") {
      // Emit the tag verbatim, switch back to text mode.
      out += html.slice(chunkStart, i + 1);
      inTag = false;
      chunkStart = i + 1;
      i += 1;
      continue;
    }
    i += 1;
  }
  // Trailing text after the last tag.
  if (inTag) {
    out += html.slice(chunkStart);
  } else {
    flushText(html.length);
  }
  return out;
}
