/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx,js,jsx}"],
  theme: {
    extend: {
      colors: {
        // Cyberpunk palette — matches the CLI banner / Flutter theme.
        "neon-green":   "#00FF9F",
        "neon-magenta": "#FF00FF",
        "cyber-cyan":   "#00FFFF",
        "dim-green":    "#008250",
        "soft-grey":    "#9696A0",
        "bg-deep":      "#0A0A0F",
        "bg-panel":     "#11111A",
        "bg-elevated":  "#161623",
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
        // `font-display`. See index.html for the Google Fonts <link>.
        display: [
          "Orbitron",
          "JetBrains Mono",
          "monospace",
        ],
      },
      boxShadow: {
        "neon-green":   "0 0 12px rgba(0, 255, 159, 0.45)",
        "neon-magenta": "0 0 12px rgba(255, 0, 255, 0.45)",
        "cyber-cyan":   "0 0 12px rgba(0, 255, 255, 0.45)",
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
