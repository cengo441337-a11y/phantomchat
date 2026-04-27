import { useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import type { Contact } from "../types";

interface Props {
  /// Hex pubkey we're about to bind (already stashed in backend AppState).
  /// Shown in the header so the user can sanity-check it against an
  /// out-of-band confirmation from the sender.
  pubHex: string | null;
  contacts: Contact[];
  onClose: () => void;
  /// Caller invokes `bind_last_unbound_sender` and updates contact list.
  /// Errors must bubble (no swallow into a chat-stream system row) so
  /// this modal can render them inline where the user is looking.
  onBind: (contactLabel: string) => Promise<void> | void;
  /// Re-fetch + re-render the contact list after we create a new entry
  /// via `add_contact_from_unbound_sender`. Without this the freshly
  /// created contact wouldn't appear in the parent's list until the next
  /// boot / manual refresh.
  onContactsChanged?: () => Promise<void> | void;
}

export default function BindContactModal({
  pubHex,
  contacts,
  onClose,
  onBind,
  onContactsChanged,
}: Props) {
  const { t } = useTranslation();
  // Modal-local error + busy state — mirrors the AddContactModal fix.
  // Previously, bind failures were swallowed into a chat-stream system
  // row that the user couldn't see while focused on the modal, making
  // the modal appear to "do nothing" on backend rejection.
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Inline "create new contact" form — closes the UX gap where this
  // modal was useless if the user had no existing contact matching the
  // unknown sender. Filling both fields + submit calls
  // `add_contact_from_unbound_sender` which atomically creates the
  // entry and binds the pending pubkey to it.
  const [newLabel, setNewLabel] = useState("");
  const [newAddress, setNewAddress] = useState("");

  async function fire(contactLabel: string) {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await onBind(contactLabel);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function fireCreate() {
    if (busy) return;
    const label = newLabel.trim();
    const address = newAddress.trim();
    if (!label || !address) {
      setError(
        t("bind_modal.create_validation", {
          defaultValue: "Bitte Nickname UND Phantom-Adresse angeben.",
        }),
      );
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await invoke("add_contact_from_unbound_sender", {
        label,
        address,
      });
      if (onContactsChanged) await onContactsChanged();
      setNewLabel("");
      setNewAddress("");
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

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

        {contacts.length > 0 && (
          <>
            <div className="text-[10px] uppercase tracking-widest text-soft-grey mb-1">
              {t("bind_modal.bind_to_existing", {
                defaultValue: "An existierenden Kontakt binden:",
              })}
            </div>
            <ul className="max-h-[180px] overflow-y-auto border border-dim-green/40 rounded">
              {contacts.map(c => {
                const alreadyBound = !!c.signing_pub;
                return (
                  <li
                    key={c.label}
                    onClick={() => void fire(c.label)}
                    aria-disabled={busy}
                    className={
                      "px-3 py-2 border-b border-dim-green/20 last:border-b-0 transition-colors " +
                      (busy
                        ? "cursor-wait opacity-60"
                        : "cursor-pointer hover:bg-neon-magenta/10")
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
          </>
        )}

        {/* Create-new-contact form — always visible. Useful (a) when the
            user has zero contacts and (b) when none of them match the
            unbound sender (the previous flow was: cancel modal, open
            Add-Contact, paste address, submit, re-trigger bind). */}
        <div className="mt-4 border border-dim-green/40 rounded p-3 bg-bg-deep/60">
          <div className="text-[10px] uppercase tracking-widest text-neon-green mb-2">
            {t("bind_modal.create_section_title", {
              defaultValue: "Oder: neuen Kontakt anlegen + verknüpfen",
            })}
          </div>
          <input
            type="text"
            value={newLabel}
            onChange={e => setNewLabel(e.target.value)}
            placeholder={t("bind_modal.create_label_placeholder", {
              defaultValue: "Nickname (z.B. 'alice')",
            })}
            disabled={busy}
            className="w-full mb-2 px-2 py-1.5 bg-bg-deep border border-dim-green/40 rounded text-cyber-cyan font-mono text-xs focus:outline-none focus:border-neon-magenta"
          />
          <input
            type="text"
            value={newAddress}
            onChange={e => setNewAddress(e.target.value)}
            placeholder={t("bind_modal.create_address_placeholder", {
              defaultValue: "phantom:<view_hex>:<spend_hex>",
            })}
            disabled={busy}
            className="w-full mb-2 px-2 py-1.5 bg-bg-deep border border-dim-green/40 rounded text-cyber-cyan font-mono text-xs focus:outline-none focus:border-neon-magenta"
          />
          <button
            onClick={() => void fireCreate()}
            disabled={busy || !newLabel.trim() || !newAddress.trim()}
            className="neon-button text-xs disabled:opacity-40"
          >
            {busy
              ? t("bind_modal.creating", { defaultValue: "anlegen…" })
              : t("bind_modal.create_button", {
                  defaultValue: "Anlegen + Binden",
                })}
          </button>
        </div>

        {error && (
          <div className="mt-3 text-xs text-neon-magenta">! {error}</div>
        )}

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
