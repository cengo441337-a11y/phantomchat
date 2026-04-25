import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import type { Contact, ConversationState } from "../types";

interface Props {
  contacts: Contact[];
  activeLabel: string | null;
  onSelect: (label: string) => void;
  onAddClick: () => void;
  /// True when the backend has stashed an unbound sealed-sender pubkey
  /// waiting for `bind_last_unbound_sender`. Renders a clickable banner.
  hasUnboundSender?: boolean;
  onBindClick?: () => void;
  /// Wave 8G — per-conversation pin/archive state. Sourced from
  /// `conversation_state.json` via the parent. The pane uses these to:
  ///   - pin pinned contacts to the top of the live list
  ///   - shunt archived contacts into the collapsible "Archiv (N)" section
  ///   - draw a 📌 icon on pinned rows
  conversationState?: Record<string, ConversationState>;
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
  conversationState,
}: Props) {
  const { t } = useTranslation();

  // ── Wave 8G: split into live + archived; pinned float to top ────────
  // The split is purely a render-side concern — the source of truth is
  // `conversation_state.json` in the backend, mirrored into the
  // `conversationState` prop by App.tsx.
  const { liveContacts, archivedContacts } = useMemo(() => {
    const live: Contact[] = [];
    const archived: Contact[] = [];
    for (const c of contacts) {
      if (conversationState?.[c.label]?.archived) {
        archived.push(c);
      } else {
        live.push(c);
      }
    }
    // Pinned-first inside the live list. Stable ordering otherwise so the
    // user's existing mental model of contact order isn't shuffled.
    live.sort((a, b) => {
      const aPinned = conversationState?.[a.label]?.pinned ? 1 : 0;
      const bPinned = conversationState?.[b.label]?.pinned ? 1 : 0;
      return bPinned - aPinned;
    });
    return { liveContacts: live, archivedContacts: archived };
  }, [contacts, conversationState]);

  /// Archive section open/closed — collapsed by default so the live list
  /// dominates the view. Persists in component state only (not localStorage)
  /// since the user's intent here is contextual, not durable.
  const [archiveOpen, setArchiveOpen] = useState(false);

  // ── Right-click / kebab context menu ──────────────────────────────────
  const [menu, setMenu] = useState<{
    label: string;
    x: number;
    y: number;
  } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menu) return;
    function onDown(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenu(null);
      }
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setMenu(null);
    }
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu]);

  function openMenu(e: React.MouseEvent, label: string) {
    e.preventDefault();
    e.stopPropagation();
    setMenu({ label, x: e.clientX, y: e.clientY });
  }

  function callBackend(cmd: string, contactLabel: string) {
    void invoke(cmd, { contactLabel }).catch(err =>
      console.warn(`${cmd} failed:`, err),
    );
    setMenu(null);
  }

  function renderContactRow(c: Contact) {
    const active = c.label === activeLabel;
    const unbound = !c.signing_pub;
    const pinned = !!conversationState?.[c.label]?.pinned;
    const archived = !!conversationState?.[c.label]?.archived;
    return (
      <li
        key={c.label}
        onClick={() => onSelect(c.label)}
        onContextMenu={e => openMenu(e, c.label)}
        className={
          "cursor-pointer px-3 py-2 pc-contact-row " +
          (active ? "is-active" : "")
        }
      >
        <div className="flex items-center gap-2">
          <span className={active ? "text-neon-green" : "text-soft-grey"}>
            {active ? "▶" : "·"}
          </span>
          {pinned && (
            <span
              className="text-neon-magenta text-xs"
              aria-label="pinned"
              title={t("conversation.archive.pin_button")}
            >
              {"\u{1F4CC}"}
            </span>
          )}
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
          {/* Kebab opens the same menu as right-click; placed at the row
              edge so it's reachable from a trackpad without secondary-
              click gymnastics. */}
          <button
            onClick={e => {
              e.stopPropagation();
              openMenu(e, c.label);
            }}
            className="ml-1 text-soft-grey hover:text-neon-magenta text-xs px-1"
            aria-label={t("conversation.archive.menu_aria")}
            title={t("conversation.archive.menu_aria")}
          >
            ⋯
          </button>
        </div>
        <div className="pl-5 text-[10px] text-soft-grey font-mono truncate">
          {shortAddr(c.address)}
          {archived && (
            <span className="ml-2 text-soft-grey/60">[archived]</span>
          )}
        </div>
      </li>
    );
  }

  // The current state of `menu.label`'s ConversationState drives the
  // pin/archive labels (toggle vs add). Sourced fresh on each render so
  // the menu reflects in-flight `conversation_state_changed` events.
  const menuState =
    menu && conversationState ? conversationState[menu.label] : undefined;
  const menuArchived = !!menuState?.archived;
  const menuPinned = !!menuState?.pinned;

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
        {liveContacts.length === 0 && archivedContacts.length === 0 && (
          <li className="px-3 py-3 text-soft-grey text-xs italic">
            {t("contacts_pane.empty")}
          </li>
        )}
        {liveContacts.map(renderContactRow)}

        {archivedContacts.length > 0 && (
          <li>
            <button
              onClick={() => setArchiveOpen(o => !o)}
              className="w-full text-left px-3 py-2 mt-2 border-t border-dim-green/30 text-soft-grey text-[10px] uppercase tracking-widest hover:text-neon-green flex items-center justify-between"
            >
              <span>
                {archiveOpen ? "▾" : "▸"}{" "}
                {t("conversation.archive.section_header", {
                  count: archivedContacts.length,
                })}
              </span>
            </button>
            {archiveOpen && (
              <ul>{archivedContacts.map(renderContactRow)}</ul>
            )}
          </li>
        )}
      </ul>

      {menu && (
        <div
          ref={menuRef}
          className="fixed z-50 bg-bg-panel border border-neon-magenta/60 rounded-md shadow-neon-magenta py-1 text-xs"
          style={{
            left: Math.min(menu.x, window.innerWidth - 200),
            top: Math.min(menu.y, window.innerHeight - 120),
            minWidth: "180px",
          }}
        >
          <button
            onClick={() =>
              callBackend(
                menuPinned ? "unpin_conversation" : "pin_conversation",
                menu.label,
              )
            }
            className="w-full text-left px-3 py-1.5 hover:bg-neon-magenta/10 text-cyber-cyan"
          >
            {"\u{1F4CC} "}
            {menuPinned
              ? t("conversation.archive.unpin_button")
              : t("conversation.archive.pin_button")}
          </button>
          <button
            onClick={() =>
              callBackend(
                menuArchived
                  ? "unarchive_conversation"
                  : "archive_conversation",
                menu.label,
              )
            }
            className="w-full text-left px-3 py-1.5 hover:bg-neon-magenta/10 text-neon-green"
          >
            {menuArchived
              ? t("conversation.archive.unarchive_button")
              : t("conversation.archive.archive_button")}
          </button>
        </div>
      )}
    </aside>
  );
}
