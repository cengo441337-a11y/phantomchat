import { useTranslation } from "react-i18next";
import type { Contact } from "../types";

interface Props {
  /// Hex pubkey we're about to bind (already stashed in backend AppState).
  /// Shown in the header so the user can sanity-check it against an
  /// out-of-band confirmation from the sender.
  pubHex: string | null;
  contacts: Contact[];
  onClose: () => void;
  /// Caller invokes `bind_last_unbound_sender` and updates contact list.
  onBind: (contactLabel: string) => Promise<void> | void;
}

export default function BindContactModal({
  pubHex,
  contacts,
  onClose,
  onBind,
}: Props) {
  const { t } = useTranslation();
  return (
    <div
      className="fixed inset-0 bg-black/60 flex items-center justify-center z-50 pc-modal-backdrop"
      onClick={onClose}
    >
      <div
        className="border border-neon-magenta rounded-md w-[520px] max-w-[90%] p-5 pc-modal-panel"
        onClick={e => e.stopPropagation()}
      >
        <h2 className="text-neon-magenta font-bold uppercase tracking-widest text-sm mb-2 font-display pc-brand-glow-magenta">
          {t("bind_modal.title")}
        </h2>
        <p className="text-soft-grey text-xs mb-4">
          {t("bind_modal.description")}
        </p>

        {pubHex && (
          <div className="mb-4 px-3 py-2 bg-bg-deep border border-dim-green/40 rounded">
            <div className="text-[10px] uppercase tracking-widest text-soft-grey mb-1">
              {t("bind_modal.pubkey_label")}
            </div>
            <div className="font-mono text-xs text-cyber-cyan break-all">
              {pubHex}
            </div>
          </div>
        )}

        <ul className="max-h-[280px] overflow-y-auto border border-dim-green/40 rounded">
          {contacts.length === 0 && (
            <li className="px-3 py-3 text-soft-grey text-xs italic">
              {t("bind_modal.empty")}
            </li>
          )}
          {contacts.map(c => {
            const alreadyBound = !!c.signing_pub;
            return (
              <li
                key={c.label}
                onClick={() => void onBind(c.label)}
                className={
                  "cursor-pointer px-3 py-2 border-b border-dim-green/20 last:border-b-0 transition-colors " +
                  "hover:bg-neon-magenta/10"
                }
                title={
                  alreadyBound
                    ? t("bind_modal.rebind_warning_title")
                    : ""
                }
              >
                <div className="flex items-center justify-between">
                  <span className="text-cyber-cyan font-bold">{c.label}</span>
                  {alreadyBound && (
                    <span className="text-[10px] text-yellow-400 uppercase tracking-wider">
                      {t("bind_modal.will_rebind")}
                    </span>
                  )}
                </div>
                <div className="text-[10px] text-soft-grey font-mono truncate">
                  {c.address}
                </div>
              </li>
            );
          })}
        </ul>

        <div className="flex justify-end gap-2 mt-5">
          <button
            onClick={onClose}
            className="text-soft-grey hover:text-neon-green text-sm uppercase tracking-wider px-3 py-2"
          >
            {t("bind_modal.cancel")}
          </button>
        </div>
      </div>
    </div>
  );
}
