import { useTranslation } from "react-i18next";
import type { ConnectionStatus } from "../types";

interface Props {
  scanned: number;
  decrypted: number;
  relay: string;
  connection: ConnectionStatus;
  /// Optional gear-button click handler — opens the SettingsPanel overlay.
  /// Optional so existing callers / screenshots that don't wire it stay
  /// shape-compatible.
  onOpenSettings?: () => void;
}

function shortRelay(url: string): string {
  return url.replace(/^wss?:\/\//, "");
}

export default function StatusFooter({
  scanned,
  decrypted,
  relay,
  connection,
  onOpenSettings,
}: Props) {
  const { t } = useTranslation();
  const { dotClass, textClass, label, pulseClass } = pillStyle(connection, t);
  return (
    <footer className="flex items-center justify-between px-4 py-1.5 text-[11px] border-t border-dim-green/40 bg-bg-panel/80 backdrop-blur-sm">
      <div className="flex items-center gap-3 text-soft-grey">
        <span
          className="flex items-center gap-1 px-2 py-0.5 rounded border border-dim-green/40 bg-bg-deep"
          title={t("status_footer.subscription_title", { status: label })}
        >
          <span className={`${dotClass} text-base leading-none ${pulseClass}`}>●</span>
          <span className={`${textClass} uppercase tracking-wider`}>
            {label}
          </span>
        </span>
        <span className="text-dim-green">│</span>
        <span>
          <kbd className="text-neon-green">Enter</kbd> {t("status_footer.kbd_send")}
        </span>
        <span className="text-dim-green">│</span>
        <span>
          <kbd className="text-neon-green">+ new</kbd> {t("status_footer.kbd_add_contact")}
        </span>
      </div>
      <div className="flex items-center gap-3">
        <span className="text-cyber-cyan">
          {t("status_footer.scanned_decrypted", { scanned, decrypted })}
        </span>
        <span className="text-soft-grey">│</span>
        <span className="text-neon-magenta">{shortRelay(relay)}</span>
        {onOpenSettings && (
          <button
            onClick={onOpenSettings}
            title={t("status_footer.settings_title")}
            aria-label={t("status_footer.settings_aria")}
            className="text-soft-grey hover:text-neon-green transition-colors px-1"
          >
            ⚙
          </button>
        )}
      </div>
    </footer>
  );
}

function pillStyle(
  status: ConnectionStatus,
  t: (key: string) => string,
): {
  dotClass: string;
  textClass: string;
  label: string;
  /// Tailwind animation class hooked into the keyframes from styles.css —
  /// `pc-pulse-{connecting,connected,disconnected}`.
  pulseClass: string;
} {
  switch (status) {
    case "connected":
      return {
        dotClass: "text-neon-green",
        textClass: "text-neon-green",
        label: t("status_footer.connected"),
        pulseClass: "animate-pulse-connected",
      };
    case "disconnected":
      return {
        dotClass: "text-red-500",
        textClass: "text-red-400",
        label: t("status_footer.disconnected"),
        pulseClass: "animate-pulse-disconnected",
      };
    case "connecting":
    default:
      return {
        dotClass: "text-soft-grey",
        textClass: "text-soft-grey",
        label: t("status_footer.connecting"),
        pulseClass: "animate-pulse-connecting",
      };
  }
}
