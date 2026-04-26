import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

/// Three-up theme picker. Reads + writes `phantomchat-theme` in
/// localStorage and mirrors the value onto `<html data-theme="…">`,
/// which is what styles.css's `[data-theme="…"]` blocks key off.
///
/// On any change we also dispatch a `theme-change` CustomEvent on
/// `window` so other components (e.g. an in-memory canvas chart)
/// can re-paint with the new accent palette without remounting.
///
/// Usable from:
///   - SettingsPanel  → "Erscheinungsbild" section
///   - OnboardingWizard → first-run preference step
/// Both render the same component; pick-state is shared across the
/// app via the localStorage backing store.

export type ThemeId = "cyberpunk" | "light" | "corporate";
const STORAGE_KEY = "phantomchat-theme";
const DEFAULT_THEME: ThemeId = "cyberpunk";

interface ThemeSpec {
  id: ThemeId;
  /// One-of icon glyph rendered above the label. Pure unicode so we
  /// don't pull a heavy icon lib for three buttons.
  glyph: string;
  /// Five-band hex preview (bg / panel / fg-dim / primary / secondary).
  /// Rendered inline as an SVG — see `Swatch` below.
  swatch: [string, string, string, string, string];
}

const THEMES: ThemeSpec[] = [
  {
    id: "cyberpunk",
    glyph: "\u{1F303}", // night-with-stars
    swatch: ["#070710", "#11111A", "#9696A0", "#00FF9F", "#FF00FF"],
  },
  {
    id: "light",
    glyph: "\u{2600}",  // sun
    swatch: ["#F8F8FA", "#FFFFFF", "#6A6A75", "#008060", "#B23BB2"],
  },
  {
    id: "corporate",
    glyph: "\u{1F3E2}", // office building
    swatch: ["#FAFAFA", "#FFFFFF", "#707075", "#2E5BFF", "#6E7383"],
  },
];

function readStoredTheme(): ThemeId {
  try {
    const v = window.localStorage.getItem(STORAGE_KEY);
    if (v === "cyberpunk" || v === "light" || v === "corporate") return v;
  } catch {
    /* localStorage unavailable (private mode) — fall through */
  }
  return DEFAULT_THEME;
}

/// Apply a theme without re-rendering React: set the data-theme attr,
/// persist, broadcast. Exported so callers (e.g. main.tsx) can pre-apply
/// the saved theme before React mounts to avoid a flash.
export function applyTheme(theme: ThemeId): void {
  document.documentElement.setAttribute("data-theme", theme);
  try {
    window.localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    /* swallow — best-effort persistence */
  }
  window.dispatchEvent(
    new CustomEvent("theme-change", { detail: { theme } }),
  );
}

export default function ThemeSwitcher() {
  const { t } = useTranslation();
  const [active, setActive] = useState<ThemeId>(() => readStoredTheme());

  // Mirror the active theme onto <html> on every change. We also do this
  // once on mount in case main.tsx didn't pre-apply (e.g. in tests).
  useEffect(() => {
    applyTheme(active);
  }, [active]);

  return (
    <div className="grid grid-cols-3 gap-2">
      {THEMES.map(spec => {
        const isActive = spec.id === active;
        return (
          <button
            key={spec.id}
            type="button"
            onClick={() => setActive(spec.id)}
            aria-pressed={isActive}
            className={
              "flex flex-col items-center gap-1 rounded-md border px-2 py-2 transition-all duration-150 " +
              (isActive
                ? "border-neon-green bg-neon-green/10 shadow-neon-green"
                : "border-dim-green/50 hover:border-neon-green/70 hover:bg-neon-green/5")
            }
            title={t(`settings.theme.${spec.id}_hint`)}
          >
            <span className="text-lg leading-none" aria-hidden="true">
              {spec.glyph}
            </span>
            <Swatch bands={spec.swatch} />
            <span
              className={
                "text-[10px] uppercase tracking-wider " +
                (isActive ? "text-neon-green" : "text-soft-grey")
              }
            >
              {t(`settings.theme.${spec.id}_name`)}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/// 60×40 SVG with five horizontal stripes — quick-read preview of the
/// theme's palette. Inline so it survives PurgeCSS / theme switches.
function Swatch({ bands }: { bands: [string, string, string, string, string] }) {
  const stripeH = 40 / bands.length;
  return (
    <svg
      width={60}
      height={40}
      viewBox="0 0 60 40"
      role="img"
      aria-hidden="true"
      className="rounded-sm border border-soft-grey/30"
    >
      {bands.map((color, i) => (
        <rect
          key={i}
          x={0}
          y={i * stripeH}
          width={60}
          height={stripeH}
          fill={color}
        />
      ))}
    </svg>
  );
}
