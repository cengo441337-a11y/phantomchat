import { useState } from "react";
import { useTranslation } from "react-i18next";

interface Props {
  onClose: () => void;
  onSubmit: (label: string, address: string) => Promise<void> | void;
}

export default function AddContactModal({ onClose, onSubmit }: Props) {
  const { t } = useTranslation();
  const [label, setLabel] = useState("");
  const [address, setAddress] = useState("");
  const [submitting, setSubmitting] = useState(false);
  // Modal-local error so the user sees Tauri-side `add_contact` rejection
  // (invalid format, duplicate label, save failure) inline instead of
  // having the parent's pushSystem write into a chat stream they're not
  // looking at, which made the modal appear to "do nothing".
  const [error, setError] = useState<string | null>(null);

  async function fire() {
    if (!label.trim() || !address.trim() || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await onSubmit(label.trim(), address.trim());
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
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
        <h2 className="text-neon-magenta font-bold uppercase tracking-widest text-sm mb-4 font-display pc-brand-glow-magenta">
          {t("add_contact_modal.title")}
        </h2>

        <div className="space-y-3">
          <div>
            <label className="block text-xs text-soft-grey uppercase tracking-wider mb-1">
              {t("add_contact_modal.label")}
            </label>
            <input
              autoFocus
              type="text"
              value={label}
              onChange={e => setLabel(e.target.value)}
              placeholder={t("add_contact_modal.label_placeholder")}
              className="neon-input w-full"
            />
          </div>
          <div>
            <label className="block text-xs text-soft-grey uppercase tracking-wider mb-1">
              {t("add_contact_modal.address_label")}
            </label>
            <input
              type="text"
              value={address}
              onChange={e => setAddress(e.target.value)}
              placeholder={t("add_contact_modal.address_placeholder")}
              className="neon-input w-full text-xs"
            />
          </div>
          {error && (
            <div className="text-xs text-neon-magenta">! {error}</div>
          )}
        </div>

        <div className="flex justify-end gap-2 mt-5">
          <button
            onClick={onClose}
            className="text-soft-grey hover:text-neon-green text-sm uppercase tracking-wider px-3 py-2"
          >
            {t("add_contact_modal.cancel")}
          </button>
          <button
            onClick={() => void fire()}
            disabled={!label.trim() || !address.trim() || submitting}
            className="neon-button disabled:opacity-40"
          >
            {submitting ? t("add_contact_modal.saving") : t("add_contact_modal.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
