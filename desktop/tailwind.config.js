/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx,js,jsx}"],
  theme: {
    extend: {
      colors: {
        // Theme tokens — wired to CSS custom properties defined per
        // [data-theme="…"] in styles.css. Each value uses the `rgb(... / <alpha-value>)`
        // shape so Tailwind's `text-neon-green/60` opacity shorthand can
        // substitute the alpha channel. The underlying RGB triplet vars
        // (`--pc-rgb-primary` etc.) are also defined per theme block, so
        // every existing component class adopts the active palette
        // automatically with NO per-component edits.

        // New canonical aliases (prefer these in fresh code):
        "pc-bg":         "rgb(var(--pc-rgb-bg, 7 7 16) / <alpha-value>)",
        "pc-panel":      "var(--pc-bg-panel)",
        "pc-elevated":   "var(--pc-bg-elevated)",
        "pc-fg":         "var(--pc-fg)",
        "pc-fg-dim":     "var(--pc-fg-dim)",
        "pc-primary":    "rgb(var(--pc-rgb-primary) / <alpha-value>)",
        "pc-secondary":  "rgb(var(--pc-rgb-secondary) / <alpha-value>)",
        "pc-tertiary":   "rgb(var(--pc-rgb-tertiary) / <alpha-value>)",

        // Legacy aliases — every existing component class points here.
        // These are the SAME triplets as the canonical aliases above,
        // so a switch to data-theme="light" cascades to every neon-*
        // class without per-component edits.
        "neon-green":    "rgb(var(--pc-rgb-primary) / <alpha-value>)",
        "neon-magenta":  "rgb(var(--pc-rgb-secondary) / <alpha-value>)",
        "cyber-cyan":    "rgb(var(--pc-rgb-tertiary) / <alpha-value>)",
        "dim-green":     "rgb(var(--pc-rgb-dim) / <alpha-value>)",
        "soft-grey":     "rgb(var(--pc-rgb-fg-dim) / <alpha-value>)",
        "bg-deep":       "var(--pc-bg-deep)",
        "bg-panel":      "var(--pc-bg-panel)",
        "bg-elevated":   "var(--pc-bg-elevated)",
      },
      fontFamily: {
        mono: [
          "JetBrains Mono",
          "Fira Code",
          "Menlo",
          "Consolas",
          "monospace",
        ],
        // Display font for headers / pane titles. Used sparingly via
        // `font-display`. The actual resolved family is theme-driven via
        // the `--pc-font-display` CSS var (see styles.css), so corporate
        // theme renders Inter while cyberpunk/light render Orbitron. This
        // entry is the Tailwind-config fallback for any code that uses
        // the family directly via `font-display`.
        display: [
          "Orbitron",
          "Inter",
          "JetBrains Mono",
          "monospace",
        ],
      },
      boxShadow: {
        // rgba() can't take a CSS-var directly, so we use the rgb-triplet
        // vars defined in styles.css (`--pc-rgb-primary` etc.). Result:
        // shadows recolor automatically with the active theme.
        "neon-green":   "0 0 12px rgba(var(--pc-rgb-primary), 0.45)",
        "neon-magenta": "0 0 12px rgba(var(--pc-rgb-secondary), 0.45)",
        "cyber-cyan":   "0 0 12px rgba(var(--pc-rgb-tertiary), 0.45)",
      },
      animation: {
        // Connection-pill pulses (3 states). Keyframes are defined in
        // styles.css under @layer utilities so they survive PurgeCSS.
        "pulse-connecting":   "pc-pulse-connecting 2s ease-in-out infinite",
        "pulse-connected":    "pc-pulse-connected 3s ease-in-out infinite",
        "pulse-disconnected": "pc-pulse-disconnected 1.5s ease-in-out infinite",
        // Message-row enter animations (run-once on mount).
        "row-in-incoming":    "pc-row-in-incoming 0.2s ease-out both",
        "row-in-outgoing":    "pc-row-in-outgoing 0.2s ease-out both",
        // Modal fade-in.
        "modal-in":           "pc-modal-in 0.2s ease-out both",
        // Cursor blink for the input bar.
        "cursor-blink":       "pc-cursor-blink 1.2s steps(2, end) infinite",
      },
    },
  },
  plugins: [],
};
