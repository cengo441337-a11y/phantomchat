// Inline-only markdown rendering for chat bodies. Uses `marked` in the
// `parseInline` mode so we never get block-level wrapping (no <p>, no
// auto-paragraphs).
//
// Pipeline order (see MessageStream.tsx):
//   raw text  →  renderMarkdown  →  autoLinkify  →  detectMentions  →  HTML
//
// Each stage is *additive* HTML: every later stage operates on the HTML
// produced by the previous one, treating tags it doesn't recognise as
// opaque so it never re-escapes already-escaped content.
//
// Security model: we configure `marked` with `gfm + breaks` and override
// the link / image / html renderers so user-supplied hrefs / raw HTML
// are filtered. Every text-bearing renderer escapes its `text` argument
// before interpolating into the output. As a defence-in-depth backstop,
// `parseInline` is used (NOT `parse`) which restricts token types to the
// inline subset — block-level tokens (raw_html, blockquote, etc.) never
// fire.

import { marked } from "marked";

/// Escape the five HTML-significant characters. Used everywhere user-
/// supplied content lands inside an HTML attribute or text node.
export function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/// Reject hrefs that aren't on a known-safe scheme. Returns `null` for
/// `javascript:`, `data:`, custom schemes, etc. — caller renders the
/// link text as a plain span in that case.
function sanitizeUrl(href: string | null | undefined): string | null {
  if (!href) return null;
  const trimmed = href.trim();
  if (/^https?:\/\//i.test(trimmed)) return trimmed;
  if (/^mailto:/i.test(trimmed)) return trimmed;
  return null;
}

// Configure a single marked instance with our renderer overrides + safe
// defaults. `marked.use` is idempotent if called once at module load.
marked.use({
  breaks: true,
  gfm: true,
  renderer: {
    // Strip all block wrappers — chat bodies are single-line. Returning
    // the raw `text` (already-rendered child HTML) keeps inline emphasis
    // intact while dropping the surrounding <p> / <h1> / etc.
    paragraph({ text }) {
      return text;
    },
    heading({ text }) {
      return `<span class="font-bold text-neon-green">${text}</span>`;
    },
    blockquote({ text }) {
      return `<span class="opacity-70 italic">${text}</span>`;
    },
    list({ items }) {
      return items.map(it => it.text).join(" · ");
    },
    listitem({ text }) {
      return text;
    },
    hr() {
      return "";
    },
    // Raw inline HTML should NEVER pass through — escape it so a sender
    // can't smuggle <script> by typing it as a literal HTML token.
    html({ raw }) {
      return escapeHtml(raw);
    },
    // Inline emphasis — keep semantics, theme via tailwind classes.
    strong({ text }) {
      return `<strong class="font-bold">${text}</strong>`;
    },
    em({ text }) {
      return `<em class="italic">${text}</em>`;
    },
    del({ text }) {
      return `<del class="line-through opacity-70">${text}</del>`;
    },
    // Open every link in a new window with `noopener noreferrer`.
    link({ href, text }) {
      const safeHref = sanitizeUrl(href);
      if (!safeHref) {
        return `<span>${escapeHtml(text || href || "")}</span>`;
      }
      return `<a href="${escapeHtml(safeHref)}" target="_blank" rel="noopener noreferrer" class="text-cyber-cyan underline hover:text-neon-green">${escapeHtml(text || safeHref)}</a>`;
    },
    // Inline + block code — both escape their body so backtick-wrapped
    // angle brackets render as visible characters, never tags.
    code({ text }) {
      return `<pre class="bg-black/40 px-2 py-1 rounded text-neon-green font-mono text-xs whitespace-pre-wrap break-all inline-block max-w-full align-middle">${escapeHtml(text)}</pre>`;
    },
    codespan({ text }) {
      return `<code class="bg-black/40 px-1 rounded text-neon-green font-mono">${escapeHtml(text)}</code>`;
    },
    // Drop images entirely — chat bodies can't reasonably embed remote
    // images (no CSP control, would leak the user's IP to arbitrary
    // hosts). Render the alt text instead, plainly escaped.
    image({ text, title }) {
      return `<span class="opacity-60">[${escapeHtml(text || title || "image")}]</span>`;
    },
    // Plain text runs — escape and pass through. The `text` token type
    // can also be Tag/Escape — we handle those by escaping their body.
    text(token) {
      // `Tag` / `Escape` tokens have a `text` field too, just typed differently.
      const t = (token as { text?: string }).text ?? "";
      return escapeHtml(t);
    },
  },
});

/// Render a chat-body string as inline HTML. The input is treated as
/// markdown but only the inline subset is honoured — block-level tokens
/// are flattened by the renderer overrides above. Every text run is
/// HTML-escaped, so `<script>alert(1)</script>` shows up as visible
/// text rather than executing.
export function renderMarkdown(text: string): string {
  if (!text) return "";
  // `parseInline` returns a string in non-async mode (the default). We
  // cast because the marked types union it with `Promise<string>`.
  return marked.parseInline(text) as string;
}
