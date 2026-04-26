import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";

interface Props {
  /// The active conversation peer's label, or `null` when no contact is
  /// selected. The TTL dropdown is disabled in the no-contact case.
  activeLabel: string | null;
  /// Fired with the new TTL (in seconds, or `null` to disable) so the
  /// parent can echo a system message into the message stream. The
  /// component itself handles the backend round-trip — the parent only
  /// needs to react to the user-visible side effect.
  onTtlChanged?: (ttlSecs: number | null) => void;
}

/// Five preset TTL slots — Off + four positive durations matching what
/// Signal/Threema/Wire surface in their disappearing-messages menus.
/// `null` means "auto-purge disabled" for the conversation.
const TTL_PRESETS: Array<{ value: number | null; key: string }> = [
  { value: null, key: "off" },
  { value: 5 * 60, key: "min5" },
  { value: 60 * 60, key: "hour1" },
  { value: 24 * 60 * 60, key: "hour24" },
  { value: 7 * 24 * 60 * 60, key: "day7" },
];

export default function ConversationHeader({ activeLabel, onTtlChanged }: Props) {
  const { t } = useTranslation();
  // `null` = no TTL set yet for this conversation. Hydrated from the
  // backend via `get_disappearing_ttl` whenever the active contact
  // changes, so the dropdown reflects the persisted setting on tab
  // switches without having to round-trip per render.
  const [ttl, setTtl] = useState<number | null>(null);
  const [pending, setPending] = useState(false);

  useEffect(() => {
    if (!activeLabel) {
      setTtl(null);
      return;
    }
    void (async () => {
      try {
        const cur = await invoke<number | null>("get_disappearing_ttl", {
          contactLabel: activeLabel,
        });
        setTtl(cur);
      } catch {
        setTtl(null);
      }
    })();
  }, [activeLabel]);

  async function handleChange(value: number | null) {
    if (!activeLabel || pending) return;
    setPending(true);
    try {
      await invoke("set_disappearing_ttl", {
        contactLabel: activeLabel,
        ttlSecs: value,
      });
      setTtl(value);
      onTtlChanged?.(value);
    } catch (e) {
      console.warn("set_disappearing_ttl failed:", e);
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="flex items-center justify-between px-4 py-1 text-xs border-b border-dim-green/30 bg-bg-panel/40">
      <div className="flex items-center gap-2">
        {/* Clock icon stays visible even when the dropdown is "off" so the
            control is discoverable. Dim when off, bright cyan when active. */}
        <span
          aria-hidden="true"
          className={
            "text-base leading-none " +
            (ttl !== null ? "text-cyber-cyan" : "text-soft-grey/50")
          }
          title={
            ttl !== null
              ? t("messages.disappearing.active_title", { secs: ttl })
              : t("messages.disappearing.inactive_title")
          }
        >
          {"\u{23F1}"}
        </span>
        <label className="text-soft-grey uppercase tracking-widest font-display">
          {t("messages.disappearing.label")}
        </label>
        <select
          value={ttl === null ? "off" : String(ttl)}
          onChange={e => {
            const v = e.target.value;
            void handleChange(v === "off" ? null : Number(v));
          }}
          disabled={!activeLabel || pending}
          className="bg-bg-elevated text-neon-green border border-dim-green/60 rounded px-2 py-0.5 font-mono text-xs disabled:opacity-50"
        >
          {TTL_PRESETS.map(p => (
            <option
              key={p.key}
              value={p.value === null ? "off" : String(p.value)}
            >
              {t(`messages.disappearing.preset_${p.key}`)}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}
