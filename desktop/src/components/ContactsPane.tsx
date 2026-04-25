import { useTranslation } from "react-i18next";
import type { Contact } from "../types";

interface Props {
  contacts: Contact[];
  activeLabel: string | null;
  onSelect: (label: string) => void;
  onAddClick: () => void;
  /// True when the backend has stashed an unbound sealed-sender pubkey
  /// waiting for `bind_last_unbound_sender`. Renders a clickable banner.
  hasUnboundSender?: boolean;
  onBindClick?: () => void;
}

function shortAddr(s: string): string {
  const prefix = s.startsWith("phantomx:") ? "phantomx:" : "phantom:";
  const body = s.replace(/^phantomx?:/, "");
  if (body.length < 12) return s;
  return `${prefix}${body.slice(0, 6)}…${body.slice(-4)}`;
}

export default function ContactsPane({
  contacts,
  activeLabel,
  onSelect,
  onAddClick,
  hasUnboundSender,
  onBindClick,
}: Props) {
  const { t } = useTranslation();
  return (
    <aside
      tabIndex={-1}
      className="w-[280px] shrink-0 flex flex-col border-r border-dim-green/40 bg-bg-panel/85 backdrop-blur-[1px] pc-pane"
    >
      <div className="flex items-center justify-between px-3 py-2 border-b border-dim-green/40">
        <span className="text-neon-green text-xs uppercase tracking-widest font-display">
          {t("contacts_pane.header")}
        </span>
        <button
          onClick={onAddClick}
          className="text-neon-magenta text-xs hover:text-neon-green transition-all duration-150 hover:[text-shadow:0_0_8px_rgba(255,0,255,0.7)]"
          title={t("contacts_pane.add_new_title")}
        >
          {t("contacts_pane.add_new")}
        </button>
      </div>

      {hasUnboundSender && (
        <button
          onClick={onBindClick}
          className="text-left px-3 py-2 border-b border-neon-magenta/50 bg-neon-magenta/10 hover:bg-neon-magenta/20 transition-colors"
          title={t("contacts_pane.unbound_pending_title")}
        >
          <div className="text-[11px] text-neon-magenta font-bold uppercase tracking-widest">
            🔓 {t("contacts_pane.unbound_pending_title")}
          </div>
          <div className="text-[10px] text-soft-grey mt-0.5">
            {t("contacts_pane.unbound_pending_hint")}
          </div>
        </button>
      )}

      <ul className="flex-1 overflow-y-auto">
        {contacts.length === 0 && (
          <li className="px-3 py-3 text-soft-grey text-xs italic">
            {t("contacts_pane.empty")}
          </li>
        )}
        {contacts.map(c => {
          const active = c.label === activeLabel;
          const unbound = !c.signing_pub;
          return (
            <li
              key={c.label}
              onClick={() => onSelect(c.label)}
              className={
                "cursor-pointer px-3 py-2 pc-contact-row " +
                (active ? "is-active" : "")
              }
            >
              <div className="flex items-center gap-2">
                <span className={active ? "text-neon-green" : "text-soft-grey"}>
                  {active ? "▶" : "·"}
                </span>
                <span
                  className={
                    "font-bold truncate " +
                    (active ? "text-neon-green" : "text-cyber-cyan")
                  }
                >
                  {c.label}
                </span>
                {unbound && (
                  <span
                    className="ml-auto text-[9px] uppercase tracking-wider text-yellow-400/90 border border-yellow-400/40 rounded px-1 py-0.5"
                    title={t("contacts_pane.unbound_badge_title")}
                  >
                    {t("contacts_pane.unbound_badge")}
                  </span>
                )}
              </div>
              <div className="pl-5 text-[10px] text-soft-grey font-mono truncate">
                {shortAddr(c.address)}
              </div>
            </li>
          );
        })}
      </ul>
    </aside>
  );
}
