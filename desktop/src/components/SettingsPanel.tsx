import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import AddressQR from "./AddressQR";
import type { AuditEntry, PrivacyConfigDto, UpdateInfo } from "../types";

interface Props {
  onClose: () => void;
}

interface IdentityFields {
  address: string;
  fingerprintShort: string;
  signingPubShort: string;
}

/// Settings overlay — opened from the gear button in StatusFooter. Sections:
///   - Identity        (address, short fingerprint, signing-pub head, copy/backup, language toggle)
///   - Privacy         (DailyUse / MaximumStealth + proxy)
///   - Relays          (editable list, save/discard)
///   - About           (version + brand + check-for-updates)
///   - Audit Log       (last 100 entries with category badges + filter + export path)
///   - Danger Zone     (collapsed wipe-all-data with two-step DELETE confirm)
///
/// Wire format mirrors AddContactModal: full-screen dim layer + centered
/// neon-magenta-bordered card. Closing on backdrop click is intentional —
/// nothing the user does in this panel commits without an explicit button.
export default function SettingsPanel({ onClose }: Props) {
  const { t, i18n } = useTranslation();

  // ── Identity ────────────────────────────────────────────────────────────
  const [identity, setIdentity] = useState<IdentityFields | null>(null);
  const [identityErr, setIdentityErr] = useState<string | null>(null);
  const [backupPath, setBackupPath] = useState<string | null>(null);
  const [qrSvg, setQrSvg] = useState<string | null>(null);
  /// Editable display name persisted via `set_my_label` / `get_my_label`.
  /// Saved with a 600 ms debounce so per-keystroke writes don't pummel
  /// `me.json`. The "saved" pill flashes for a beat each time the
  /// debounced timer fires.
  const [myLabel, setMyLabel] = useState<string>("");
  const [myLabelSaved, setMyLabelSaved] = useState<boolean>(false);
  const myLabelTimer = useRef<number | null>(null);

  // ── Language ────────────────────────────────────────────────────────────
  // The dropdown writes to localStorage via the i18next-browser-languagedetector
  // cache so the choice survives restarts. "Auto" wipes the cache so the next
  // launch re-detects via `navigator.language`.
  const [langSel, setLangSel] = useState<"auto" | "en" | "de">(() => {
    const cached = window.localStorage.getItem("i18nextLng");
    if (cached === "en" || cached === "de") return cached;
    return "auto";
  });

  // ── Relays ──────────────────────────────────────────────────────────────
  const [relays, setRelays] = useState<string[]>([]);
  const [relaysOriginal, setRelaysOriginal] = useState<string[]>([]);
  const [relaysSaving, setRelaysSaving] = useState(false);
  const [relaysMsg, setRelaysMsg] = useState<string | null>(null);

  // ── Privacy ─────────────────────────────────────────────────────────────
  const [privacy, setPrivacy] = useState<PrivacyConfigDto>({
    mode: "DailyUse",
    proxy_addr: "127.0.0.1:9050",
    proxy_kind: "Tor",
  });
  const [privacyOriginal, setPrivacyOriginal] = useState<PrivacyConfigDto>({
    mode: "DailyUse",
    proxy_addr: "127.0.0.1:9050",
    proxy_kind: "Tor",
  });
  const [privacySaving, setPrivacySaving] = useState(false);
  const [privacyMsg, setPrivacyMsg] = useState<string | null>(null);

  // ── About ───────────────────────────────────────────────────────────────
  const [version, setVersion] = useState<string>("?.?.?");
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateInstalling, setUpdateInstalling] = useState(false);
  const [updateMsg, setUpdateMsg] = useState<string | null>(null);

  // ── Audit Log ───────────────────────────────────────────────────────────
  const [auditEntries, setAuditEntries] = useState<AuditEntry[]>([]);
  const [auditErr, setAuditErr] = useState<string | null>(null);
  const [auditFilter, setAuditFilter] = useState<string>("__all__");
  const [auditExportedPath, setAuditExportedPath] = useState<string | null>(null);

  // ── Danger Zone ─────────────────────────────────────────────────────────
  const [dangerOpen, setDangerOpen] = useState(false);
  const [wipeStage, setWipeStage] = useState<"idle" | "confirm">("idle");
  const [wipeText, setWipeText] = useState("");
  const [wipeErr, setWipeErr] = useState<string | null>(null);

  useEffect(() => {
    (async () => {
      try {
        const addr = await invoke<string>("get_address");
        const stripped = addr.replace(/^phantomx?:/, "");
        const fingerprintShort = stripped.slice(0, 16);
        const parts = stripped.split(":");
        const signingPubShort = (parts[1] ?? "").slice(0, 32);
        setIdentity({ address: addr, fingerprintShort, signingPubShort });
        try {
          const svg = await invoke<string>("address_qr_svg");
          setQrSvg(svg);
        } catch {
          /* leave qr null */
        }
      } catch (e) {
        setIdentityErr(String(e));
      }
      try {
        const list = await invoke<string[]>("list_relays");
        setRelays(list);
        setRelaysOriginal(list);
      } catch (e) {
        setRelaysMsg(t("settings.relays.msg_load_failed", { error: String(e) }));
      }
      try {
        const v = await invoke<string>("get_app_version");
        setVersion(v);
      } catch {
        /* leave default */
      }
      try {
        const lbl = await invoke<string>("get_my_label");
        setMyLabel(lbl ?? "");
      } catch {
        /* leave blank */
      }
      try {
        const p = await invoke<PrivacyConfigDto>("get_privacy_config");
        setPrivacy(p);
        setPrivacyOriginal(p);
      } catch (e) {
        setPrivacyMsg(t("settings.privacy.msg_save_failed", { error: String(e) }));
      }
      // Load audit entries on mount so the section renders immediately.
      try {
        const entries = await invoke<AuditEntry[]>("read_audit_log", { limit: 100 });
        setAuditEntries(entries);
      } catch (e) {
        setAuditErr(String(e));
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /// Debounced save for the display-name input. Triggered on every
  /// `setMyLabel`. The 600 ms delay balances responsiveness against
  /// disk-write churn — anything shorter and a fast typist racks up
  /// dozens of `me.json` writes per word.
  useEffect(() => {
    if (myLabelTimer.current !== null) {
      window.clearTimeout(myLabelTimer.current);
    }
    myLabelTimer.current = window.setTimeout(() => {
      void (async () => {
        try {
          await invoke("set_my_label", { label: myLabel });
          setMyLabelSaved(true);
          window.setTimeout(() => setMyLabelSaved(false), 1200);
        } catch {
          /* swallow — UI doesn't surface a per-keystroke error */
        }
      })();
    }, 600);
    return () => {
      if (myLabelTimer.current !== null) {
        window.clearTimeout(myLabelTimer.current);
      }
    };
  }, [myLabel]);

  function changeLanguage(value: "auto" | "en" | "de") {
    setLangSel(value);
    if (value === "auto") {
      // Drop the cached override so the LanguageDetector falls back to
      // navigator.language on the next module init. Active session keeps
      // the current locale until the next reload — that's fine, the user
      // just opted into "follow browser" and will see it on next launch.
      window.localStorage.removeItem("i18nextLng");
    } else {
      void i18n.changeLanguage(value);
    }
  }

  async function handleBackup() {
    try {
      const p = await invoke<string>("export_keyfile");
      setBackupPath(p);
    } catch (e) {
      setBackupPath(`error: ${String(e)}`);
    }
  }

  function updateRelay(idx: number, value: string) {
    setRelays(rs => rs.map((r, i) => (i === idx ? value : r)));
  }
  function removeRelay(idx: number) {
    setRelays(rs => rs.filter((_, i) => i !== idx));
  }
  function addRelayRow() {
    setRelays(rs => [...rs, ""]);
  }

  async function saveRelays() {
    setRelaysSaving(true);
    setRelaysMsg(null);
    try {
      const cleaned = relays.map(r => r.trim()).filter(r => r.length > 0);
      await invoke("set_relays", { urls: cleaned });
      setRelays(cleaned);
      setRelaysOriginal(cleaned);
      setRelaysMsg(t("settings.relays.msg_saved"));
    } catch (e) {
      setRelaysMsg(t("settings.relays.msg_save_failed", { error: String(e) }));
    } finally {
      setRelaysSaving(false);
    }
  }

  function discardRelays() {
    setRelays(relaysOriginal);
    setRelaysMsg(t("settings.relays.msg_discarded"));
  }

  /// Persist the Privacy section then trigger `restart_listener` so the
  /// new mode takes effect immediately.
  async function savePrivacy() {
    setPrivacySaving(true);
    setPrivacyMsg(null);
    try {
      await invoke("set_privacy_config", { cfg: privacy });
      setPrivacyOriginal(privacy);
      setPrivacyMsg(
        privacy.mode === "MaximumStealth"
          ? t("settings.privacy.msg_reconnecting", { kind: privacy.proxy_kind })
          : t("settings.privacy.msg_reconnecting_plain"),
      );
      try {
        await invoke("restart_listener");
        setPrivacyMsg(t("settings.privacy.msg_saved"));
      } catch (e) {
        setPrivacyMsg(t("settings.privacy.msg_saved_restart_failed", { error: String(e) }));
      }
    } catch (e) {
      setPrivacyMsg(t("settings.privacy.msg_save_failed", { error: String(e) }));
    } finally {
      setPrivacySaving(false);
    }
  }

  function discardPrivacy() {
    setPrivacy(privacyOriginal);
    setPrivacyMsg(t("settings.privacy.msg_discarded"));
  }

  async function checkForUpdates() {
    setUpdateChecking(true);
    setUpdateMsg(null);
    try {
      const info = await invoke<UpdateInfo>("check_for_updates");
      setUpdateInfo(info);
      if (!info.available) {
        setUpdateMsg(t("settings.about.up_to_date"));
      }
    } catch (e) {
      setUpdateMsg(t("settings.about.update_failed", { error: String(e) }));
    } finally {
      setUpdateChecking(false);
    }
  }

  async function installUpdate() {
    setUpdateInstalling(true);
    setUpdateMsg(null);
    try {
      await invoke("install_update");
      // On Windows + macOS the installer takes over and exits the app.
      // On Linux (AppImage) the user must manually relaunch.
    } catch (e) {
      setUpdateMsg(t("settings.about.update_failed", { error: String(e) }));
    } finally {
      setUpdateInstalling(false);
    }
  }

  async function reloadAudit() {
    try {
      const entries = await invoke<AuditEntry[]>("read_audit_log", { limit: 100 });
      setAuditEntries(entries);
      setAuditErr(null);
    } catch (e) {
      setAuditErr(String(e));
    }
  }

  async function exportAudit() {
    try {
      const p = await invoke<string>("export_audit_log");
      setAuditExportedPath(p);
    } catch (e) {
      setAuditExportedPath(`error: ${String(e)}`);
    }
  }

  async function fireWipe() {
    if (wipeText.trim().toUpperCase() !== "DELETE") {
      setWipeErr(t("settings.danger.must_type_delete"));
      return;
    }
    try {
      await invoke("wipe_all_data");
    } catch (e) {
      setWipeErr(t("settings.danger.wipe_failed", { error: String(e) }));
    }
  }

  const relaysDirty =
    relays.length !== relaysOriginal.length ||
    relays.some((r, i) => r !== relaysOriginal[i]);

  const privacyDirty =
    privacy.mode !== privacyOriginal.mode ||
    privacy.proxy_addr !== privacyOriginal.proxy_addr ||
    privacy.proxy_kind !== privacyOriginal.proxy_kind;

  // Distinct categories for the audit-log filter dropdown — derived from
  // the current rendered set so a backend extension surfaces automatically.
  const auditCategories = Array.from(
    new Set(auditEntries.map(e => e.category)),
  ).sort();
  const auditVisible =
    auditFilter === "__all__"
      ? auditEntries
      : auditEntries.filter(e => e.category === auditFilter);

  return (
    <div
      className="fixed inset-0 bg-black/70 flex items-center justify-center z-50"
      onClick={onClose}
    >
      <div
        className="bg-bg-panel border border-neon-magenta shadow-neon-magenta rounded-md w-[640px] max-w-[92%] max-h-[88vh] overflow-y-auto p-6"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-center justify-between mb-5">
          <h2 className="text-neon-magenta font-bold uppercase tracking-widest text-sm">
            {t("settings.title")}
          </h2>
          <button
            onClick={onClose}
            className="text-soft-grey hover:text-neon-green text-sm uppercase tracking-wider px-2"
            aria-label={t("settings.close")}
          >
            ✕
          </button>
        </div>

        {/* ── Identity ────────────────────────────────────────────────── */}
        <section className="mb-6">
          <h3 className="text-neon-green text-xs uppercase tracking-widest mb-2">
            {t("settings.identity.title")}
          </h3>
          {identityErr && (
            <div className="text-xs text-neon-magenta">{identityErr}</div>
          )}
          {identity && (
            <div className="space-y-3 text-xs">
              <AddressQR svg={qrSvg} address={identity.address} />
              <Row
                label={t("settings.identity.fingerprint")}
                mono
                value={identity.fingerprintShort}
              />
              <Row
                label={t("settings.identity.signing_pub")}
                mono
                value={identity.signingPubShort}
              />
              <div className="flex flex-col gap-1">
                <span className="text-soft-grey uppercase tracking-wider text-[10px]">
                  {t("settings.identity.display_name_label")}
                </span>
                <div className="flex gap-2 items-center">
                  <input
                    type="text"
                    value={myLabel}
                    onChange={e => setMyLabel(e.target.value)}
                    placeholder={t("settings.identity.display_name_placeholder")}
                    className="neon-input flex-1 text-xs"
                  />
                  {myLabelSaved && (
                    <span className="text-cyber-cyan text-[10px] uppercase tracking-wider">
                      {t("settings.identity.display_name_saved")}
                    </span>
                  )}
                </div>
                <span className="text-soft-grey text-[10px]">
                  {t("settings.identity.display_name_hint")}
                </span>
              </div>

              {/* Language sub-section */}
              <div className="flex flex-col gap-1 pt-2 border-t border-dim-green/30">
                <span className="text-soft-grey uppercase tracking-wider text-[10px]">
                  {t("settings.language.title")}
                </span>
                <select
                  value={langSel}
                  onChange={e => changeLanguage(e.target.value as "auto" | "en" | "de")}
                  className="neon-input text-xs"
                >
                  <option value="auto">{t("settings.language.auto")}</option>
                  <option value="en">{t("settings.language.en")}</option>
                  <option value="de">{t("settings.language.de")}</option>
                </select>
              </div>

              <div className="flex gap-2 pt-2">
                <button onClick={handleBackup} className="neon-button text-xs">
                  {t("settings.identity.backup_button")}
                </button>
              </div>
              {backupPath && (
                <div className="text-xs text-cyber-cyan break-all pt-1">
                  {backupPath}
                </div>
              )}
            </div>
          )}
        </section>

        {/* ── Privacy ────────────────────────────────────────────────── */}
        <section className="mb-6">
          <h3 className="text-neon-green text-xs uppercase tracking-widest mb-2">
            {t("settings.privacy.title")}
          </h3>
          <div className="space-y-3 text-xs">
            <div className="flex flex-col gap-1">
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="radio"
                  name="privacy-mode"
                  checked={privacy.mode === "DailyUse"}
                  onChange={() =>
                    setPrivacy(p => ({ ...p, mode: "DailyUse" }))
                  }
                />
                <span className="text-neon-green">{t("settings.privacy.daily_use")}</span>
                <span className="text-soft-grey text-[10px]">
                  {t("settings.privacy.daily_use_hint")}
                </span>
              </label>
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="radio"
                  name="privacy-mode"
                  checked={privacy.mode === "MaximumStealth"}
                  onChange={() =>
                    setPrivacy(p => ({ ...p, mode: "MaximumStealth" }))
                  }
                />
                <span className="text-neon-magenta">{t("settings.privacy.max_stealth")}</span>
                <span className="text-soft-grey text-[10px]">
                  {t("settings.privacy.max_stealth_hint")}
                </span>
              </label>
            </div>

            {privacy.mode === "MaximumStealth" && (
              <div className="space-y-2 pl-5 border-l border-neon-magenta/40">
                <div className="flex flex-col gap-1">
                  <span className="text-soft-grey uppercase tracking-wider text-[10px]">
                    {t("settings.privacy.proxy_addr_label")}
                  </span>
                  <input
                    type="text"
                    value={privacy.proxy_addr}
                    onChange={e =>
                      setPrivacy(p => ({ ...p, proxy_addr: e.target.value }))
                    }
                    placeholder="127.0.0.1:9050"
                    className="neon-input text-xs"
                  />
                </div>
                <div className="flex flex-col gap-1">
                  <span className="text-soft-grey uppercase tracking-wider text-[10px]">
                    {t("settings.privacy.network_label")}
                  </span>
                  <div className="flex items-center gap-4">
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="radio"
                        name="proxy-kind"
                        checked={privacy.proxy_kind === "Tor"}
                        onChange={() =>
                          setPrivacy(p => ({ ...p, proxy_kind: "Tor" }))
                        }
                      />
                      <span className="text-neon-green">Tor</span>
                    </label>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="radio"
                        name="proxy-kind"
                        checked={privacy.proxy_kind === "Nym"}
                        onChange={() =>
                          setPrivacy(p => ({ ...p, proxy_kind: "Nym" }))
                        }
                      />
                      <span className="text-neon-green">Nym</span>
                    </label>
                  </div>
                </div>
              </div>
            )}

            <div className="flex gap-2 mt-3 items-center">
              <button
                onClick={() => void savePrivacy()}
                disabled={!privacyDirty || privacySaving}
                className="neon-button text-xs disabled:opacity-40"
              >
                {privacySaving ? t("settings.privacy.saving") : t("settings.privacy.save")}
              </button>
              <button
                onClick={discardPrivacy}
                disabled={!privacyDirty || privacySaving}
                className="text-soft-grey hover:text-neon-green text-xs uppercase tracking-wider px-3 py-2 disabled:opacity-40"
              >
                {t("settings.privacy.discard")}
              </button>
              {privacyMsg && (
                <span className="text-xs text-cyber-cyan ml-2">
                  {privacyMsg}
                </span>
              )}
            </div>
          </div>
        </section>

        {/* ── Relays ─────────────────────────────────────────────────── */}
        <section className="mb-6">
          <h3 className="text-neon-green text-xs uppercase tracking-widest mb-2">
            {t("settings.relays.title")}
          </h3>
          <div className="space-y-2">
            {relays.map((url, idx) => (
              <div key={idx} className="flex gap-2 items-center">
                <input
                  type="text"
                  value={url}
                  onChange={e => updateRelay(idx, e.target.value)}
                  placeholder="wss://relay.example.org"
                  className="neon-input flex-1 text-xs"
                />
                <button
                  onClick={() => removeRelay(idx)}
                  className="text-soft-grey hover:text-neon-magenta text-xs uppercase tracking-wider px-2 py-2"
                >
                  {t("settings.relays.remove")}
                </button>
              </div>
            ))}
            <button
              onClick={addRelayRow}
              className="text-cyber-cyan hover:text-neon-green text-xs uppercase tracking-wider"
            >
              {t("settings.relays.add")}
            </button>
          </div>
          <div className="flex gap-2 mt-3 items-center">
            <button
              onClick={() => void saveRelays()}
              disabled={!relaysDirty || relaysSaving}
              className="neon-button text-xs disabled:opacity-40"
            >
              {relaysSaving ? t("settings.relays.saving") : t("settings.relays.save")}
            </button>
            <button
              onClick={discardRelays}
              disabled={!relaysDirty}
              className="text-soft-grey hover:text-neon-green text-xs uppercase tracking-wider px-3 py-2 disabled:opacity-40"
            >
              {t("settings.relays.discard")}
            </button>
            {relaysMsg && (
              <span className="text-xs text-cyber-cyan ml-2">{relaysMsg}</span>
            )}
          </div>
        </section>

        {/* ── About ───────────────────────────────────────────────────── */}
        <section className="mb-6">
          <h3 className="text-neon-green text-xs uppercase tracking-widest mb-2">
            {t("settings.about.title")}
          </h3>
          <div className="text-xs text-soft-grey space-y-2">
            <div>
              {t("settings.about.version_label")}{" "}
              <span className="text-neon-green">{version}</span>
            </div>
            <div>
              {t("settings.about.by_line")}{" "}
              <span className="text-neon-magenta">DC INFOSEC</span> ·
              dc-infosec.de
            </div>
            <div className="flex gap-2 items-center pt-1">
              <button
                onClick={() => void checkForUpdates()}
                disabled={updateChecking || updateInstalling}
                className="neon-button text-xs disabled:opacity-40"
              >
                {updateChecking
                  ? t("settings.about.checking_updates")
                  : t("settings.about.check_updates_button")}
              </button>
              {updateInfo?.available && updateInfo.version && (
                <>
                  <span className="text-cyber-cyan text-xs">
                    {t("settings.about.update_available", { version: updateInfo.version })}
                  </span>
                  <button
                    onClick={() => void installUpdate()}
                    disabled={updateInstalling}
                    className="neon-button text-xs disabled:opacity-40"
                  >
                    {updateInstalling
                      ? t("settings.about.installing_update")
                      : t("settings.about.install_update_button")}
                  </button>
                </>
              )}
              {updateMsg && (
                <span className="text-xs text-cyber-cyan ml-2">{updateMsg}</span>
              )}
            </div>
          </div>
        </section>

        {/* ── Audit Log ───────────────────────────────────────────────── */}
        <section className="mb-6">
          <div className="flex items-center justify-between mb-2">
            <h3 className="text-neon-green text-xs uppercase tracking-widest">
              {t("settings.audit.title")}
            </h3>
            <div className="flex items-center gap-2">
              <span className="text-soft-grey uppercase tracking-wider text-[10px]">
                {t("settings.audit.filter_label")}
              </span>
              <select
                value={auditFilter}
                onChange={e => setAuditFilter(e.target.value)}
                className="neon-input text-[10px] py-0.5"
              >
                <option value="__all__">{t("settings.audit.filter_all")}</option>
                {auditCategories.map(cat => (
                  <option key={cat} value={cat}>
                    {cat}
                  </option>
                ))}
              </select>
              <button
                onClick={() => void reloadAudit()}
                className="text-cyber-cyan hover:text-neon-green text-xs px-1"
                title="reload"
              >
                ⟳
              </button>
              <button
                onClick={() => void exportAudit()}
                className="neon-button text-xs"
              >
                {t("settings.audit.export_button")}
              </button>
            </div>
          </div>
          {auditExportedPath && (
            <div className="text-xs text-cyber-cyan break-all mb-2">
              {t("settings.audit.exported_path", { path: auditExportedPath })}
            </div>
          )}
          {auditErr && (
            <div className="text-xs text-neon-magenta mb-2">
              {t("settings.audit.load_failed", { error: auditErr })}
            </div>
          )}
          {auditVisible.length === 0 ? (
            <div className="text-soft-grey italic text-xs">
              {t("settings.audit.empty")}
            </div>
          ) : (
            <div className="border border-dim-green/40 rounded max-h-72 overflow-y-auto">
              <table className="w-full text-[10px] font-mono">
                <thead className="bg-bg-deep sticky top-0">
                  <tr className="text-soft-grey uppercase tracking-wider">
                    <th className="text-left px-2 py-1">{t("settings.audit.col_ts")}</th>
                    <th className="text-left px-2 py-1">{t("settings.audit.col_category")}</th>
                    <th className="text-left px-2 py-1">{t("settings.audit.col_event")}</th>
                    <th className="text-left px-2 py-1">{t("settings.audit.col_details")}</th>
                  </tr>
                </thead>
                <tbody>
                  {auditVisible.map((entry, i) => (
                    <tr key={i} className="border-t border-dim-green/20 align-top">
                      <td className="px-2 py-1 text-soft-grey whitespace-nowrap">
                        {entry.ts}
                      </td>
                      <td className="px-2 py-1">
                        <CategoryBadge category={entry.category} />
                      </td>
                      <td className="px-2 py-1 text-neon-green">{entry.event}</td>
                      <td className="px-2 py-1">
                        <pre className="text-soft-grey whitespace-pre-wrap break-all text-[10px]">
                          {Object.keys(entry.details ?? {}).length === 0
                            ? "{}"
                            : JSON.stringify(entry.details)}
                        </pre>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>

        {/* ── Danger Zone ────────────────────────────────────────────── */}
        <section className="border border-neon-magenta/40 rounded-md p-3">
          <button
            onClick={() => setDangerOpen(o => !o)}
            className="flex items-center justify-between w-full text-neon-magenta text-xs uppercase tracking-widest"
          >
            <span>{t("settings.danger.title")}</span>
            <span>{dangerOpen ? "▾" : "▸"}</span>
          </button>
          {dangerOpen && (
            <div className="mt-3 space-y-2 text-xs">
              <p className="text-soft-grey">
                {t("settings.danger.description")}
              </p>
              {wipeStage === "idle" ? (
                <button
                  onClick={() => {
                    setWipeStage("confirm");
                    setWipeErr(null);
                  }}
                  className="neon-button text-xs border-neon-magenta/70 text-neon-magenta hover:bg-neon-magenta/10"
                >
                  {t("settings.danger.wipe_button")}
                </button>
              ) : (
                <div className="space-y-2">
                  <div className="text-neon-magenta">
                    {t("settings.danger.type_delete")}
                  </div>
                  <input
                    type="text"
                    autoFocus
                    value={wipeText}
                    onChange={e => setWipeText(e.target.value)}
                    placeholder="DELETE"
                    className="neon-input w-full text-xs"
                  />
                  <div className="flex gap-2">
                    <button
                      onClick={() => void fireWipe()}
                      className="neon-button text-xs border-neon-magenta/70 text-neon-magenta hover:bg-neon-magenta/10"
                    >
                      {t("settings.danger.confirm_button")}
                    </button>
                    <button
                      onClick={() => {
                        setWipeStage("idle");
                        setWipeText("");
                        setWipeErr(null);
                      }}
                      className="text-soft-grey hover:text-neon-green text-xs uppercase tracking-wider px-3 py-2"
                    >
                      {t("settings.danger.cancel_button")}
                    </button>
                  </div>
                  {wipeErr && (
                    <div className="text-neon-magenta">{wipeErr}</div>
                  )}
                </div>
              )}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}

interface RowProps {
  label: string;
  value: string;
  mono?: boolean;
}
function Row({ label, value, mono }: RowProps) {
  return (
    <div className="flex flex-col">
      <span className="text-soft-grey uppercase tracking-wider text-[10px]">
        {label}
      </span>
      <span
        className={
          (mono ? "font-mono " : "") +
          "text-neon-green break-all text-xs"
        }
      >
        {value}
      </span>
    </div>
  );
}

/// Color-coded category pill for the audit log table. Color choices map
/// each compliance-relevant category onto a distinct accent so an auditor
/// scanning the table can spot cross-category anomalies at a glance.
function CategoryBadge({ category }: { category: string }) {
  const styles: Record<string, string> = {
    identity: "border-neon-green text-neon-green",
    contact: "border-cyber-cyan text-cyber-cyan",
    mls: "border-neon-magenta text-neon-magenta",
    relay: "border-yellow-400 text-yellow-400",
    privacy: "border-orange-400 text-orange-400",
    data: "border-red-400 text-red-400",
  };
  const cls = styles[category] ?? "border-soft-grey text-soft-grey";
  return (
    <span className={`inline-block border px-1 py-0.5 rounded text-[10px] uppercase tracking-widest ${cls}`}>
      {category}
    </span>
  );
}
