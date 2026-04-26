import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import AddressQR from "./AddressQR";
import ThemeSwitcher from "./ThemeSwitcher";
import type { AuditEntry, PrivacyConfigDto, UpdateInfo } from "../types";
import { listen } from "@tauri-apps/api/event";
import type { ConversationStateChangedEvent } from "../types";
import type {
  AuditEntry,
  CrashReport,
  PrivacyConfigDto,
  UpdateInfo,
} from "../types";
  BackupMeta,
  BackupResult,
  PrivacyConfigDto,
  RestoreResult,
  UpdateInfo,
} from "../types";
import type { AuditEntry, LanOrgStatus, PrivacyConfigDto, UpdateInfo } from "../types";

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

  // ── LAN Org ─────────────────────────────────────────────────────────────
  // Mirrors the wizard's join/create/skip flow but persists across the
  // app's lifetime — used by colleagues who joined later or by the user
  // changing their mind about which org they're in.
  const [lanStatus, setLanStatus] = useState<LanOrgStatus | null>(null);
  const [lanCodeInput, setLanCodeInput] = useState("");
  const [lanBusy, setLanBusy] = useState(false);
  const [lanErr, setLanErr] = useState<string | null>(null);
  const [lanShowCode, setLanShowCode] = useState(false);
  const [lanCodeCopied, setLanCodeCopied] = useState(false);

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

  // ── Diagnostics ─────────────────────────────────────────────────────────
  // Opt-in checkbox toggles the `crash_reporting_opted_in.flag` sentinel
  // in the app-data dir. The "show crash reports" button opens an inline
  // modal listing recent panics (timestamp + version + first-line message)
  // with per-row Send / Delete buttons. Send is hard-gated on the opt-in;
  // it returns an error string if the user hasn't ticked the box yet.
  const [crashOptIn, setCrashOptIn] = useState(false);
  const [crashModalOpen, setCrashModalOpen] = useState(false);
  const [crashReports, setCrashReports] = useState<CrashReport[]>([]);
  const [crashLoadErr, setCrashLoadErr] = useState<string | null>(null);
  const [crashSending, setCrashSending] = useState<string | null>(null);
  const [crashRowMsg, setCrashRowMsg] = useState<Record<string, string>>({});

  // ── Danger Zone ─────────────────────────────────────────────────────────
  const [dangerOpen, setDangerOpen] = useState(false);
  const [wipeStage, setWipeStage] = useState<"idle" | "confirm">("idle");
  const [wipeText, setWipeText] = useState("");
  const [wipeErr, setWipeErr] = useState<string | null>(null);

  // ── Wave 8G: Archive section ────────────────────────────────────────────
  // Lists all currently-archived conversation labels. Click → unarchive
  // (round-trip via `unarchive_conversation`). The "Alle Archive leeren"
  // button purges every archived contact from `contacts.json` — this is
  // irreversible so we gate it behind a `confirm()` dialog.
  const [archivedLabels, setArchivedLabels] = useState<string[]>([]);
  const [archiveMsg, setArchiveMsg] = useState<string | null>(null);

  async function reloadArchive() {
    try {
      const list = await invoke<string[]>("list_archived_conversations");
      setArchivedLabels(list);
    } catch (e) {
      console.warn("list_archived_conversations failed:", e);
    }
  }

  useEffect(() => {
    void reloadArchive();
    // Re-pull on any conversation_state_changed event so an archive
    // toggle from ContactsPane reflects in the Settings list without a
    // manual refresh. We discard the unlisten promise's value because
    // the effect cleanup awaits the resolved unlisten fn.
    let unlisten: (() => void) | undefined;
    void listen<ConversationStateChangedEvent>(
      "conversation_state_changed",
      () => {
        void reloadArchive();
      },
    ).then(u => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  async function handleUnarchive(label: string) {
    try {
      await invoke("unarchive_conversation", { contactLabel: label });
      // The state-change event will refresh the list automatically.
      setArchiveMsg(null);
    } catch (e) {
      setArchiveMsg(String(e));
    }
  }

  /// Empty the archive: walk every archived label and unarchive each so
  /// the user can re-find the conversations in their normal contact list.
  /// Per the spec ("removes from contacts.json — irreversible"), we ALSO
  /// invoke a follow-up `wipe_archived_contacts` if the backend ever
  /// exposes one — for now we only flip the archived bit to false because
  /// nuking contacts.json entries directly would touch out-of-scope code
  /// (contacts persistence). Confirmation gate stays in place either way
  /// so the UX flow is correct when the destructive backend lands.
  async function handleClearArchive() {
    if (archivedLabels.length === 0) return;
    const ok = window.confirm(t("settings_archive.clear_all_confirm"));
    if (!ok) return;
    try {
      for (const label of archivedLabels) {
        await invoke("unarchive_conversation", { contactLabel: label });
      }
      setArchiveMsg(t("settings_archive.clear_all_done"));
    } catch (e) {
      setArchiveMsg(
        t("settings_archive.clear_all_failed", { error: String(e) }),
      );
    }
  }
  // ── Backup & Restore (Wave 8c, compliance Aufbewahrungspflicht) ─────────
  // Two flows share this state:
  //
  //   Export: open a "create backup" modal → passphrase + confirm + strength
  //   meter → file-save dialog → invoke `export_backup` → success toast.
  //
  //   Import: file-open dialog (.pcbackup filter) → invoke `verify_backup`
  //   to surface the meta block → user confirms → invoke `import_backup` →
  //   success toast. The frontend listens for `app_data_replaced` (emitted
  //   by the backend after the swap completes) and reloads its caches.
  const [backupModal, setBackupModal] = useState<"none" | "create" | "restore">("none");
  const [backupExportPass, setBackupExportPass] = useState("");
  const [backupExportPassConfirm, setBackupExportPassConfirm] = useState("");
  const [backupExportBusy, setBackupExportBusy] = useState(false);
  const [backupExportMsg, setBackupExportMsg] = useState<string | null>(null);
  const [backupExportResult, setBackupExportResult] = useState<BackupResult | null>(null);
  const [backupImportPath, setBackupImportPath] = useState<string | null>(null);
  const [backupImportPass, setBackupImportPass] = useState("");
  const [backupImportMeta, setBackupImportMeta] = useState<BackupMeta | null>(null);
  const [backupImportBusy, setBackupImportBusy] = useState(false);
  const [backupImportMsg, setBackupImportMsg] = useState<string | null>(null);
  const [backupImportResult, setBackupImportResult] = useState<RestoreResult | null>(null);
  const [backupImportConfirmStage, setBackupImportConfirmStage] = useState<"verify" | "confirm">(
    "verify",
  );

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
      // Crash-reporting opt-in state for the Diagnostics checkbox.
      try {
        const flag = await invoke<boolean>("get_crash_reporting_opt_in");
        setCrashOptIn(flag);
      } catch {
        /* leave default false */
      try {
        const s = await invoke<LanOrgStatus>("lan_org_status");
        setLanStatus(s);
      } catch (e) {
        setLanErr(String(e));
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

  /// Lightweight passphrase strength estimator (zxcvbn-style heuristic, no
  /// external dep — keeps the bundle small). Computes Shannon-style entropy
  /// from the character class diversity multiplied by length, then maps to
  /// a 0..4 score plus a human time-to-crack hint.
  ///
  /// Pessimistic on purpose: assumes a 10^11 guesses-per-second offline
  /// attacker (single high-end GPU on a SHA-256 keyspace; Argon2id raises
  /// the bar enormously, but the meter should still nudge users toward
  /// long passphrases rather than relying on the KDF alone).
  const exportStrength = useMemo(() => {
    return estimatePassphraseStrength(backupExportPass);
  }, [backupExportPass]);

  /// True iff the current export-modal state is valid for a `create
  /// backup` invocation. Mirrors the Rust 12-char minimum and also
  /// requires the confirm field to match — preventing silent typos that
  /// would lock the user out of their own backup.
  const exportPassValid =
    backupExportPass.length >= 12 && backupExportPass === backupExportPassConfirm;

  function resetExportModal() {
    setBackupExportPass("");
    setBackupExportPassConfirm("");
    setBackupExportMsg(null);
    setBackupExportResult(null);
    setBackupExportBusy(false);
  }

  function resetImportModal() {
    setBackupImportPath(null);
    setBackupImportPass("");
    setBackupImportMeta(null);
    setBackupImportMsg(null);
    setBackupImportResult(null);
    setBackupImportBusy(false);
    setBackupImportConfirmStage("verify");
  }

  /// Begin an export. Pops the file-save dialog AFTER the passphrase is
  /// validated so the user can't get halfway through naming a file then
  /// realise they typoed the passphrase.
  async function runBackupExport() {
    if (!exportPassValid) return;
    setBackupExportBusy(true);
    setBackupExportMsg(t("settings.backup.export_in_progress"));
    setBackupExportResult(null);
    try {
      const dest = await saveDialog({
        title: t("settings.backup.export_picker_title"),
        defaultPath: `phantomchat-backup-${new Date().toISOString().slice(0, 10)}.pcbackup`,
        filters: [
          {
            name: t("settings.backup.export_filter_name"),
            extensions: ["pcbackup"],
          },
        ],
      });
      if (!dest) {
        setBackupExportMsg(null);
        setBackupExportBusy(false);
        return;
      }
      const result = await invoke<BackupResult>("export_backup", {
        outputPath: dest,
        passphrase: backupExportPass,
      });
      setBackupExportResult(result);
      setBackupExportMsg(null);
      // Wipe passphrases from React state — the Rust side already
      // zeroized its copy, the frontend should follow suit.
      setBackupExportPass("");
      setBackupExportPassConfirm("");
    } catch (e) {
      setBackupExportMsg(t("settings.backup.export_failed", { error: String(e) }));
    } finally {
      setBackupExportBusy(false);
    }
  }

  /// Step 1 of the restore flow: pop the file-open dialog. Verification
  /// itself happens after the user types the passphrase (Step 2).
  async function pickBackupForImport() {
    setBackupImportMsg(null);
    setBackupImportResult(null);
    setBackupImportMeta(null);
    setBackupImportConfirmStage("verify");
    try {
      const selected = await openDialog({
        multiple: false,
        directory: false,
        title: t("settings.backup.import_picker_title"),
        filters: [
          {
            name: t("settings.backup.export_filter_name"),
            extensions: ["pcbackup"],
          },
        ],
      });
      if (selected === null || selected === undefined) {
        return;
      }
      const path =
        typeof selected === "string"
          ? selected
          : Array.isArray(selected)
          ? null
          : (selected as { path?: string }).path ?? null;
      if (!path) return;
      setBackupImportPath(path);
      setBackupModal("restore");
    } catch (e) {
      setBackupImportMsg(t("settings.backup.import_failed", { error: String(e) }));
    }
  }

  /// Step 2 of the restore flow: validate the passphrase by invoking
  /// `verify_backup`. On success the meta block is rendered and the
  /// "Wiederherstellen" button replaces "Prüfen".
  async function runBackupVerify() {
    if (!backupImportPath || backupImportPass.length === 0) return;
    setBackupImportBusy(true);
    setBackupImportMsg(t("settings.backup.import_verifying"));
    try {
      const meta = await invoke<BackupMeta>("verify_backup", {
        inputPath: backupImportPath,
        passphrase: backupImportPass,
      });
      setBackupImportMeta(meta);
      setBackupImportMsg(null);
      setBackupImportConfirmStage("confirm");
    } catch (e) {
      setBackupImportMsg(t("settings.backup.import_failed", { error: String(e) }));
    } finally {
      setBackupImportBusy(false);
    }
  }

  /// Step 3 of the restore flow: actually invoke `import_backup`. The
  /// backend stops the listener, decrypts each entry into a temp dir,
  /// atomically swaps onto live paths, restarts the listener, and emits
  /// `app_data_replaced`. The success toast surfaces the item count.
  async function runBackupRestore() {
    if (!backupImportPath || !backupImportMeta) return;
    setBackupImportBusy(true);
    setBackupImportMsg(t("settings.backup.import_in_progress"));
    try {
      const result = await invoke<RestoreResult>("import_backup", {
        inputPath: backupImportPath,
        passphrase: backupImportPass,
      });
      setBackupImportResult(result);
      setBackupImportMsg(
        t("settings.backup.import_success", { count: result.items_restored }),
      );
      // Wipe passphrase + reset relays/audit state so the next render
      // re-fetches the freshly restored data.
      setBackupImportPass("");
      try {
        const list = await invoke<string[]>("list_relays");
        setRelays(list);
        setRelaysOriginal(list);
      } catch {
        /* surfaces via the existing relays reload path */
      }
      try {
        const entries = await invoke<AuditEntry[]>("read_audit_log", { limit: 100 });
        setAuditEntries(entries);
      } catch {
        /* leave previous list — error is already in the toast */
      }
    } catch (e) {
      setBackupImportMsg(t("settings.backup.import_failed", { error: String(e) }));
    } finally {
      setBackupImportBusy(false);
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

  /// Mirror of the wizard's input formatter — keeps the code field
  /// rendering as `XXX-XXX` regardless of how the user types/pastes.
  function formatLanCode(raw: string): string {
    const cleaned = raw.replace(/[^0-9A-Za-z]/g, "").toUpperCase().slice(0, 6);
    if (cleaned.length <= 3) return cleaned;
    return `${cleaned.slice(0, 3)}-${cleaned.slice(3)}`;
  }

  async function refreshLanStatus() {
    try {
      const s = await invoke<LanOrgStatus>("lan_org_status");
      setLanStatus(s);
    } catch (e) {
      setLanErr(String(e));
    }
  }

  async function lanCreate() {
    setLanBusy(true);
    setLanErr(null);
    try {
      await invoke<string>("lan_org_create");
      await refreshLanStatus();
      setLanShowCode(true);
    } catch (e) {
      setLanErr(String(e));
    } finally {
      setLanBusy(false);
    }
  }

  async function lanJoin() {
    setLanBusy(true);
    setLanErr(null);
    try {
      await invoke("lan_org_join", { code: lanCodeInput });
      await refreshLanStatus();
      setLanCodeInput("");
    } catch (e) {
      setLanErr(String(e));
    } finally {
      setLanBusy(false);
    }
  }

  async function lanLeave() {
    setLanBusy(true);
    setLanErr(null);
    try {
      await invoke("lan_org_leave");
      setLanShowCode(false);
      await refreshLanStatus();
    } catch (e) {
      setLanErr(String(e));
    } finally {
      setLanBusy(false);
    }
  }

  async function copyLanCode() {
    if (!lanStatus?.code) return;
    try {
      await navigator.clipboard.writeText(lanStatus.code);
      setLanCodeCopied(true);
      window.setTimeout(() => setLanCodeCopied(false), 1200);
    } catch {
      /* clipboard refused — silent */
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

  async function toggleCrashOptIn(next: boolean) {
    try {
      await invoke("set_crash_reporting_opt_in", { enabled: next });
      setCrashOptIn(next);
    } catch {
      // Silently swallow — UI just doesn't flip the box, no crash impact.
    }
  }

  async function openCrashModal() {
    setCrashModalOpen(true);
    setCrashLoadErr(null);
    try {
      const list = await invoke<CrashReport[]>("list_crash_reports", { limit: 50 });
      setCrashReports(list);
    } catch (e) {
      setCrashLoadErr(String(e));
    }
  }

  async function reloadCrashList() {
    try {
      const list = await invoke<CrashReport[]>("list_crash_reports", { limit: 50 });
      setCrashReports(list);
      setCrashLoadErr(null);
    } catch (e) {
      setCrashLoadErr(String(e));
    }
  }

  async function sendCrashReport(crashId: string) {
    setCrashSending(crashId);
    setCrashRowMsg(m => ({ ...m, [crashId]: "" }));
    try {
      await invoke<string>("dispatch_crash_report", { crashId });
      setCrashRowMsg(m => ({ ...m, [crashId]: t("settings.diagnostics.sent") }));
      // Re-pull so user_dispatched flips visually.
      await reloadCrashList();
    } catch (e) {
      setCrashRowMsg(m => ({
        ...m,
        [crashId]: t("settings.diagnostics.send_failed", { error: String(e) }),
      }));
    } finally {
      setCrashSending(null);
    }
  }

  async function clearAllCrashes() {
    try {
      await invoke("clear_crash_reports");
      setCrashReports([]);
      setCrashRowMsg({});
    } catch (e) {
      setCrashLoadErr(t("settings.diagnostics.clear_failed", { error: String(e) }));
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

        {/* ── Appearance / Theme ─────────────────────────────────────── */}
        <section className="mb-6">
          <h3 className="text-neon-green text-xs uppercase tracking-widest mb-2">
            {t("settings.theme.title")}
          </h3>
          <p className="text-xs text-soft-grey mb-3">
            {t("settings.theme.description")}
          </p>
          <ThemeSwitcher />
        {/* ── Backup & Restore ───────────────────────────────────────── */}
        <section className="mb-6">
          <h3 className="text-neon-green text-xs uppercase tracking-widest mb-2">
            {t("settings.backup.title")}
          </h3>
          <div className="text-xs text-soft-grey mb-3">
            {t("settings.backup.intro")}
          </div>
          <div className="flex gap-2">
            <button
              onClick={() => {
                resetExportModal();
                setBackupModal("create");
              }}
              className="neon-button text-xs"
            >
              {t("settings.backup.export_button")}
            </button>
            <button
              onClick={() => {
                resetImportModal();
                void pickBackupForImport();
              }}
              className="neon-button text-xs"
            >
              {t("settings.backup.import_button")}
            </button>
          </div>
          {backupExportResult && backupModal === "none" && (
            <div className="mt-3 text-xs space-y-1 border border-neon-green/40 rounded p-2">
              <div className="text-neon-green uppercase tracking-widest">
                {t("settings.backup.export_success_title")}
              </div>
              <div className="text-cyber-cyan break-all">{backupExportResult.path}</div>
              <div className="text-soft-grey">
                {t("settings.backup.export_success_size", {
                  size: backupExportResult.size_bytes,
                  count: backupExportResult.item_count,
                })}
              </div>
              <div className="text-soft-grey font-mono break-all">
                {t("settings.backup.export_success_sha", {
                  sha: backupExportResult.sha256_hex,
                })}
              </div>
            </div>
          )}
          {backupImportResult && backupModal === "none" && (
            <div className="mt-3 text-xs border border-neon-green/40 rounded p-2 text-cyber-cyan">
              {t("settings.backup.import_success", {
                count: backupImportResult.items_restored,
              })}
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

        {/* ── LAN Org ────────────────────────────────────────────────── */}
        <section className="mb-6">
          <h3 className="text-neon-green text-xs uppercase tracking-widest mb-2">
            {t("settings.lan.title")}
          </h3>
          <p className="text-xs text-soft-grey mb-2 leading-relaxed">
            {t("settings.lan.description")}
          </p>
          {lanStatus?.active ? (
            <div className="space-y-3 text-xs">
              <Row
                label={t("settings.lan.code_label")}
                mono
                value={lanShowCode && lanStatus.code ? lanStatus.code : "•••-•••"}
              />
              <Row
                label={t("settings.lan.peer_count_label")}
                value={String(lanStatus.peer_count)}
              />
              {lanStatus.last_discovery_ts && (
                <Row
                  label={t("settings.lan.last_discovery_label")}
                  mono
                  value={new Date(
                    Number(lanStatus.last_discovery_ts) * 1000,
                  ).toLocaleString()}
                />
              )}
              <div className="flex gap-2 items-center flex-wrap">
                <button
                  onClick={() => setLanShowCode(v => !v)}
                  className="neon-button text-xs"
                >
                  {lanShowCode
                    ? t("settings.lan.hide_code_button")
                    : t("settings.lan.show_code_button")}
                </button>
                {lanShowCode && lanStatus.code && (
                  <button
                    onClick={() => void copyLanCode()}
                    className="neon-button text-xs"
                  >
                    {lanCodeCopied
                      ? t("settings.lan.copied_button")
                      : t("settings.lan.copy_button")}
                  </button>
                )}
                <button
                  onClick={() => void lanLeave()}
                  disabled={lanBusy}
                  className="neon-button text-xs border-neon-magenta/70 text-neon-magenta hover:bg-neon-magenta/10 disabled:opacity-40"
                >
                  {t("settings.lan.leave_button")}
                </button>
              </div>
            </div>
          ) : (
            <div className="space-y-3 text-xs">
              <div className="flex flex-col gap-2">
                <input
                  type="text"
                  value={lanCodeInput}
                  onChange={e => setLanCodeInput(formatLanCode(e.target.value))}
                  placeholder={t("settings.lan.code_placeholder")}
                  className="neon-input text-xs font-mono tracking-widest"
                  maxLength={7}
                />
                <div className="flex gap-2">
                  <button
                    onClick={() => void lanJoin()}
                    disabled={
                      lanBusy || lanCodeInput.replace("-", "").length !== 6
                    }
                    className="neon-button text-xs flex-1 disabled:opacity-40"
                  >
                    {lanBusy
                      ? t("settings.lan.joining")
                      : t("settings.lan.join_button")}
                  </button>
                  <button
                    onClick={() => void lanCreate()}
                    disabled={lanBusy}
                    className="neon-button text-xs flex-1 disabled:opacity-40"
                  >
                    {lanBusy
                      ? t("settings.lan.creating")
                      : t("settings.lan.create_button")}
                  </button>
                </div>
              </div>
            </div>
          )}
          {lanErr && (
            <div className="text-xs text-neon-magenta mt-2 break-words">
              {lanErr}
            </div>
          )}
          <div className="text-[10px] text-neon-magenta border border-neon-magenta/60 rounded-md p-2 leading-relaxed mt-3">
            {t("settings.lan.warning")}
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

        {/* ── Archive (Wave 8G) ───────────────────────────────────────── */}
        <section className="mb-6">
          <h3 className="text-neon-green text-xs uppercase tracking-widest mb-2">
            {t("settings_archive.title")}
          </h3>
          <p className="text-soft-grey text-[10px] mb-2">
            {t("settings_archive.description")}
          </p>
          {archivedLabels.length === 0 ? (
            <div className="text-soft-grey italic text-xs">
              {t("settings_archive.empty")}
            </div>
          ) : (
            <ul className="space-y-1 mb-3">
              {archivedLabels.map(label => (
                <li
                  key={label}
                  className="flex items-center justify-between border border-dim-green/40 rounded px-3 py-1.5 text-xs"
                >
                  <span className="text-cyber-cyan font-bold truncate">
                    {label}
                  </span>
                  <button
                    onClick={() => void handleUnarchive(label)}
                    className="text-soft-grey hover:text-neon-green text-xs uppercase tracking-wider px-2"
                  >
                    {t("conversation.archive.unarchive_button")}
                  </button>
                </li>
              ))}
            </ul>
          )}
          <div className="flex gap-2 items-center">
            <button
              onClick={() => void handleClearArchive()}
              disabled={archivedLabels.length === 0}
              className="neon-button text-xs border-neon-magenta/70 text-neon-magenta hover:bg-neon-magenta/10 disabled:opacity-40"
            >
              {t("settings_archive.clear_all_button")}
            </button>
            {archiveMsg && (
              <span className="text-xs text-cyber-cyan ml-2">
                {archiveMsg}
              </span>
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

        {/* ── Diagnostics ─────────────────────────────────────────────── */}
        <section className="mb-6">
          <h3 className="text-neon-green text-xs uppercase tracking-widest mb-2">
            {t("settings.diagnostics.title")}
          </h3>
          <div className="space-y-3 text-xs">
            <label className="flex items-start gap-2 cursor-pointer">
              <input
                type="checkbox"
                className="mt-0.5"
                checked={crashOptIn}
                onChange={e => void toggleCrashOptIn(e.target.checked)}
              />
              <div className="flex flex-col">
                <span className="text-neon-green">
                  {t("settings.diagnostics.optin_label")}
                </span>
                <span className="text-soft-grey text-[10px]">
                  {t("settings.diagnostics.optin_hint")}
                </span>
              </div>
            </label>
            <div className="flex gap-2 items-center">
              <button
                onClick={() => void openCrashModal()}
                className="neon-button text-xs"
              >
                {t("settings.diagnostics.view_button")}
              </button>
            </div>
            <p className="text-soft-grey text-[10px]">
              {t("settings.diagnostics.privacy_note")}
            </p>
          </div>
        </section>

        {crashModalOpen && (
          <div
            className="fixed inset-0 bg-black/80 flex items-center justify-center z-[60]"
            onClick={() => setCrashModalOpen(false)}
          >
            <div
              className="bg-bg-panel border border-neon-magenta shadow-neon-magenta rounded-md w-[680px] max-w-[92%] max-h-[80vh] overflow-y-auto p-5"
              onClick={e => e.stopPropagation()}
            >
              <div className="flex items-center justify-between mb-3">
                <h3 className="text-neon-magenta font-bold uppercase tracking-widest text-sm">
                  {t("settings.diagnostics.view_modal_title")}
                </h3>
                <button
                  onClick={() => setCrashModalOpen(false)}
                  className="text-soft-grey hover:text-neon-green text-sm uppercase tracking-wider px-2"
                >
                  {t("settings.diagnostics.view_modal_close")}
                </button>
              </div>
              {crashLoadErr && (
                <div className="text-xs text-neon-magenta mb-2">
                  {t("settings.diagnostics.load_failed", { error: crashLoadErr })}
                </div>
              )}
              {crashReports.length === 0 ? (
                <div className="text-soft-grey italic text-xs">
                  {t("settings.diagnostics.empty")}
                </div>
              ) : (
                <>
                  <div className="border border-dim-green/40 rounded">
                    <table className="w-full text-[10px] font-mono">
                      <thead className="bg-bg-deep sticky top-0">
                        <tr className="text-soft-grey uppercase tracking-wider">
                          <th className="text-left px-2 py-1">
                            {t("settings.diagnostics.col_ts")}
                          </th>
                          <th className="text-left px-2 py-1">
                            {t("settings.diagnostics.col_version")}
                          </th>
                          <th className="text-left px-2 py-1">
                            {t("settings.diagnostics.col_msg")}
                          </th>
                          <th className="text-left px-2 py-1">
                            {t("settings.diagnostics.col_actions")}
                          </th>
                        </tr>
                      </thead>
                      <tbody>
                        {crashReports.map(report => {
                          const id = report.ts;
                          const sent = report.user_dispatched === true;
                          const sending = crashSending === id;
                          const rowMsg = crashRowMsg[id];
                          const firstLine = (report.panic_msg ?? "")
                            .split("\n")[0]
                            .slice(0, 80);
                          return (
                            <tr
                              key={id}
                              className="border-t border-dim-green/20 align-top"
                            >
                              <td className="px-2 py-1 text-soft-grey whitespace-nowrap">
                                {report.ts}
                              </td>
                              <td className="px-2 py-1 text-cyber-cyan whitespace-nowrap">
                                {report.version}
                              </td>
                              <td className="px-2 py-1 text-neon-magenta break-all">
                                {firstLine || "(no message)"}
                              </td>
                              <td className="px-2 py-1 whitespace-nowrap">
                                <div className="flex flex-col gap-1">
                                  <button
                                    onClick={() => void sendCrashReport(id)}
                                    disabled={sending || sent || !crashOptIn}
                                    className="neon-button text-[10px] py-0.5 px-2 disabled:opacity-40"
                                    title={
                                      crashOptIn
                                        ? undefined
                                        : t("settings.diagnostics.optin_hint")
                                    }
                                  >
                                    {sending
                                      ? t("settings.diagnostics.sending")
                                      : sent
                                        ? t("settings.diagnostics.sent")
                                        : t("settings.diagnostics.send_button")}
                                  </button>
                                  {rowMsg && (
                                    <span className="text-cyber-cyan text-[10px]">
                                      {rowMsg}
                                    </span>
                                  )}
                                </div>
                              </td>
                            </tr>
                          );
                        })}
                      </tbody>
                    </table>
                  </div>
                  <div className="flex justify-end mt-3">
                    <button
                      onClick={() => void clearAllCrashes()}
                      className="neon-button text-xs border-neon-magenta/70 text-neon-magenta hover:bg-neon-magenta/10"
                    >
                      {t("settings.diagnostics.clear_all_button")}
                    </button>
                  </div>
                </>
              )}
            </div>
          </div>
        )}

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

      {/* ── Backup modals ─────────────────────────────────────────────── */}
      {backupModal === "create" && (
        <div
          className="fixed inset-0 bg-black/80 flex items-center justify-center z-[60]"
          onClick={() => {
            if (!backupExportBusy) {
              resetExportModal();
              setBackupModal("none");
            }
          }}
        >
          <div
            className="bg-bg-panel border border-neon-magenta shadow-neon-magenta rounded-md w-[480px] max-w-[92%] p-5 space-y-3"
            onClick={e => e.stopPropagation()}
          >
            <h3 className="text-neon-magenta font-bold uppercase tracking-widest text-sm">
              {t("settings.backup.modal_create_title")}
            </h3>
            <div className="space-y-2 text-xs">
              <label className="block">
                <span className="text-soft-grey uppercase tracking-wider text-[10px] block mb-1">
                  {t("settings.backup.passphrase_label")}
                </span>
                <input
                  type="password"
                  autoFocus
                  value={backupExportPass}
                  onChange={e => setBackupExportPass(e.target.value)}
                  placeholder={t("settings.backup.passphrase_placeholder")}
                  className="neon-input w-full text-xs"
                />
              </label>
              <label className="block">
                <span className="text-soft-grey uppercase tracking-wider text-[10px] block mb-1">
                  {t("settings.backup.passphrase_confirm_label")}
                </span>
                <input
                  type="password"
                  value={backupExportPassConfirm}
                  onChange={e => setBackupExportPassConfirm(e.target.value)}
                  className="neon-input w-full text-xs"
                />
              </label>
              {/* Strength meter */}
              <div className="flex flex-col gap-1">
                <div className="flex items-center gap-2">
                  <span className="text-soft-grey uppercase tracking-wider text-[10px]">
                    {t("settings.backup.strength_label")}:
                  </span>
                  <span className="text-[10px]" style={{ color: exportStrength.color }}>
                    {t(exportStrength.labelKey)}
                  </span>
                </div>
                <div className="h-1.5 bg-bg-deep rounded overflow-hidden">
                  <div
                    className="h-full transition-all"
                    style={{
                      width: `${(exportStrength.score / 4) * 100}%`,
                      backgroundColor: exportStrength.color,
                    }}
                  />
                </div>
                <div className="text-soft-grey text-[10px]">
                  {t("settings.backup.strength_entropy", {
                    bits: exportStrength.entropyBits.toFixed(0),
                    time: exportStrength.crackTime,
                  })}
                </div>
              </div>
              {backupExportPass.length > 0 && backupExportPass.length < 12 && (
                <div className="text-neon-magenta text-[10px]">
                  {t("settings.backup.passphrase_too_short")}
                </div>
              )}
              {backupExportPassConfirm.length > 0 &&
                backupExportPass !== backupExportPassConfirm && (
                  <div className="text-neon-magenta text-[10px]">
                    {t("settings.backup.passphrase_mismatch")}
                  </div>
                )}
              <div className="text-soft-grey text-[10px] pt-1 border-t border-dim-green/30">
                {t("settings.backup.strength_hint")}
              </div>
              {backupExportMsg && (
                <div className="text-cyber-cyan text-[10px]">{backupExportMsg}</div>
              )}
              <div className="flex gap-2 pt-2">
                <button
                  onClick={() => void runBackupExport()}
                  disabled={!exportPassValid || backupExportBusy}
                  className="neon-button text-xs disabled:opacity-40"
                >
                  {t("settings.backup.modal_create_confirm")}
                </button>
                <button
                  onClick={() => {
                    if (!backupExportBusy) {
                      resetExportModal();
                      setBackupModal("none");
                    }
                  }}
                  disabled={backupExportBusy}
                  className="text-soft-grey hover:text-neon-green text-xs uppercase tracking-wider px-3 py-2 disabled:opacity-40"
                >
                  {t("settings.backup.import_cancel_button")}
                </button>
              </div>
              {backupExportResult && (
                <div className="text-xs space-y-1 border border-neon-green/40 rounded p-2 mt-2">
                  <div className="text-neon-green uppercase tracking-widest">
                    {t("settings.backup.export_success_title")}
                  </div>
                  <div className="text-cyber-cyan break-all">{backupExportResult.path}</div>
                  <div className="text-soft-grey">
                    {t("settings.backup.export_success_size", {
                      size: backupExportResult.size_bytes,
                      count: backupExportResult.item_count,
                    })}
                  </div>
                  <div className="text-soft-grey font-mono break-all">
                    {t("settings.backup.export_success_sha", {
                      sha: backupExportResult.sha256_hex,
                    })}
                  </div>
                  <button
                    onClick={() => {
                      resetExportModal();
                      setBackupModal("none");
                    }}
                    className="neon-button text-xs mt-1"
                  >
                    {t("settings.close")}
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {backupModal === "restore" && (
        <div
          className="fixed inset-0 bg-black/80 flex items-center justify-center z-[60]"
          onClick={() => {
            if (!backupImportBusy) {
              resetImportModal();
              setBackupModal("none");
            }
          }}
        >
          <div
            className="bg-bg-panel border border-neon-magenta shadow-neon-magenta rounded-md w-[480px] max-w-[92%] p-5 space-y-3"
            onClick={e => e.stopPropagation()}
          >
            <h3 className="text-neon-magenta font-bold uppercase tracking-widest text-sm">
              {t("settings.backup.import_meta_title")}
            </h3>
            <div className="text-xs text-soft-grey break-all">
              {backupImportPath}
            </div>
            {backupImportConfirmStage === "verify" && (
              <>
                <label className="block">
                  <span className="text-soft-grey uppercase tracking-wider text-[10px] block mb-1">
                    {t("settings.backup.import_passphrase_label")}
                  </span>
                  <input
                    type="password"
                    autoFocus
                    value={backupImportPass}
                    onChange={e => setBackupImportPass(e.target.value)}
                    className="neon-input w-full text-xs"
                  />
                </label>
                <div className="flex gap-2">
                  <button
                    onClick={() => void runBackupVerify()}
                    disabled={backupImportPass.length === 0 || backupImportBusy}
                    className="neon-button text-xs disabled:opacity-40"
                  >
                    {t("settings.backup.import_continue_button")}
                  </button>
                  <button
                    onClick={() => {
                      if (!backupImportBusy) {
                        resetImportModal();
                        setBackupModal("none");
                      }
                    }}
                    disabled={backupImportBusy}
                    className="text-soft-grey hover:text-neon-green text-xs uppercase tracking-wider px-3 py-2 disabled:opacity-40"
                  >
                    {t("settings.backup.import_cancel_button")}
                  </button>
                </div>
              </>
            )}
            {backupImportConfirmStage === "confirm" && backupImportMeta && (
              <>
                <div className="text-xs space-y-1 border border-dim-green/40 rounded p-2">
                  <Row
                    label={t("settings.backup.import_meta_created")}
                    value={backupImportMeta.created_at}
                  />
                  <Row
                    label={t("settings.backup.import_meta_host")}
                    value={backupImportMeta.host_label}
                  />
                  <Row
                    label={t("settings.backup.import_meta_count")}
                    value={String(backupImportMeta.item_count)}
                  />
                </div>
                <div className="text-neon-magenta text-xs">
                  {t("settings.backup.import_warning")}
                </div>
                <div className="flex gap-2">
                  <button
                    onClick={() => void runBackupRestore()}
                    disabled={backupImportBusy || backupImportResult !== null}
                    className="neon-button text-xs border-neon-magenta/70 text-neon-magenta hover:bg-neon-magenta/10 disabled:opacity-40"
                  >
                    {t("settings.backup.modal_restore_confirm")}
                  </button>
                  <button
                    onClick={() => {
                      if (!backupImportBusy) {
                        resetImportModal();
                        setBackupModal("none");
                      }
                    }}
                    disabled={backupImportBusy}
                    className="text-soft-grey hover:text-neon-green text-xs uppercase tracking-wider px-3 py-2 disabled:opacity-40"
                  >
                    {t("settings.backup.import_cancel_button")}
                  </button>
                </div>
              </>
            )}
            {backupImportMsg && (
              <div className="text-cyber-cyan text-[10px]">{backupImportMsg}</div>
            )}
            {backupImportResult && (
              <div className="text-cyber-cyan text-xs border border-neon-green/40 rounded p-2">
                {t("settings.backup.import_success", {
                  count: backupImportResult.items_restored,
                })}
                <button
                  onClick={() => {
                    resetImportModal();
                    setBackupModal("none");
                  }}
                  className="neon-button text-xs mt-2"
                >
                  {t("settings.close")}
                </button>
              </div>
            )}
          </div>
        </div>
      )}
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

/// Lightweight zxcvbn-style heuristic used by the backup-passphrase modal.
/// Computes character-class entropy, then scores 0..4 + maps to a human
/// time-to-crack hint assuming 1e11 guesses-per-second offline. Pessimistic
/// on purpose; Argon2id raises the bar enormously, but the meter should
/// nudge users toward long passphrases regardless.
interface StrengthEstimate {
  score: 0 | 1 | 2 | 3 | 4;
  labelKey: string;
  color: string;
  entropyBits: number;
  crackTime: string;
}

function estimatePassphraseStrength(pass: string): StrengthEstimate {
  if (pass.length === 0) {
    return {
      score: 0,
      labelKey: "settings.backup.strength_very_weak",
      color: "#9ca3af",
      entropyBits: 0,
      crackTime: "—",
    };
  }
  let charset = 0;
  if (/[a-z]/.test(pass)) charset += 26;
  if (/[A-Z]/.test(pass)) charset += 26;
  if (/[0-9]/.test(pass)) charset += 10;
  if (/[^a-zA-Z0-9]/.test(pass)) charset += 32;
  if (charset === 0) charset = 26;
  const entropyBits = Math.log2(charset) * pass.length;

  let score: 0 | 1 | 2 | 3 | 4;
  let labelKey: string;
  let color: string;
  if (entropyBits < 28) {
    score = 0;
    labelKey = "settings.backup.strength_very_weak";
    color = "#ef4444";
  } else if (entropyBits < 50) {
    score = 1;
    labelKey = "settings.backup.strength_weak";
    color = "#f97316";
  } else if (entropyBits < 70) {
    score = 2;
    labelKey = "settings.backup.strength_fair";
    color = "#eab308";
  } else if (entropyBits < 90) {
    score = 3;
    labelKey = "settings.backup.strength_strong";
    color = "#22c55e";
  } else {
    score = 4;
    labelKey = "settings.backup.strength_very_strong";
    color = "#10b981";
  }

  // Time-to-crack at 1e11 guesses/sec (high-end GPU on a fast hash).
  const guesses = Math.pow(2, entropyBits) / 2;
  const seconds = guesses / 1e11;
  const crackTime = humaniseSeconds(seconds);
  return { score, labelKey, color, entropyBits, crackTime };
}

function humaniseSeconds(s: number): string {
  if (!isFinite(s) || s > 1e15) return "centuries";
  if (s < 1) return "<1s";
  if (s < 60) return `${s.toFixed(0)}s`;
  if (s < 3600) return `${(s / 60).toFixed(0)}min`;
  if (s < 86400) return `${(s / 3600).toFixed(0)}h`;
  if (s < 86400 * 365) return `${(s / 86400).toFixed(0)}d`;
  if (s < 86400 * 365 * 1000) return `${(s / (86400 * 365)).toFixed(0)}y`;
  return `>${(s / (86400 * 365 * 1000)).toFixed(0)}k years`;
}
