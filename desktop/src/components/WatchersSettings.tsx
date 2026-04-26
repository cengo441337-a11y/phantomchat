import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  Watcher,
  WatcherFireResult,
  WatcherMode,
  WatcherSchedule,
} from "../types";

interface Props {
  /// Allowlist from the parent AiBridgeConfig — gates the target-contact
  /// dropdown. Watchers can only target labels on the bridge allow-list
  /// (defensive — same backend gate also rejects mistyped targets).
  allowlist: string[];
}

interface DraftWatcher {
  id?: string;
  name: string;
  scheduleType: "interval" | "cron";
  intervalSecs: number;
  cronExpr: string;
  command: string;
  target_contact: string;
  mode: WatcherMode;
  enabled: boolean;
}

const EMPTY_DRAFT: DraftWatcher = {
  name: "",
  scheduleType: "interval",
  intervalSecs: 300,
  cronExpr: "0 0 9 * * *",
  command: "",
  target_contact: "",
  mode: "raw",
  enabled: true,
};

/// Wave 11E — Proactive Watchers settings. Lives inside the AI Bridge
/// section of `SettingsPanel`. The UI flow:
///   - List of existing watchers (cards with last_run_at + status + Run/Edit/Delete)
///   - "+ New watcher" button → inline modal/form
///   - Cron expression validator that calls back into Rust
///
/// Persistence is one-shot per save: each form action invokes a single
/// Tauri command and re-pulls the list. We don't try to be clever with
/// optimistic updates because the ground truth lives in
/// `<app_data>/ai_bridge_watchers.json`, not React state.
export default function WatchersSettings({ allowlist }: Props) {
  const { t } = useTranslation();
  const [watchers, setWatchers] = useState<Watcher[]>([]);
  const [editing, setEditing] = useState<DraftWatcher | null>(null);
  const [cronVerification, setCronVerification] = useState<{
    ok: boolean;
    msg: string;
  } | null>(null);
  const [fireResults, setFireResults] = useState<Record<string, string>>({});
  /// Per-watcher in-flight set for the "Run now" button. Without this,
  /// rapid clicks would launch parallel subprocesses for the same
  /// watcher (10 clicks = 10 concurrent shell runs of the same command).
  const [runningSet, setRunningSet] = useState<Set<string>>(() => new Set());
  /// Per-row inline error map for delete + toggle failures. Previously
  /// these were `console.warn`-only, so a backend rejection (file lock,
  /// permission denied, missing watcher id after a race) silently
  /// dropped — the UI just appeared to "do nothing".
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});

  async function reload() {
    try {
      const list = await invoke<Watcher[]>("ai_bridge_list_watchers");
      setWatchers(list);
    } catch (e) {
      console.warn("ai_bridge_list_watchers failed:", e);
    }
  }

  useEffect(() => {
    void reload();
    let unlisten: (() => void) | undefined;
    void listen<string>("ai_bridge_watcher_fired", () => {
      void reload();
    }).then(fn => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  function draftToSchedule(d: DraftWatcher): WatcherSchedule {
    return d.scheduleType === "interval"
      ? { type: "interval", secs: Math.max(1, d.intervalSecs) }
      : { type: "cron", expr: d.cronExpr };
  }

  function watcherToDraft(w: Watcher): DraftWatcher {
    return {
      id: w.id,
      name: w.name,
      scheduleType: w.schedule.type === "interval" ? "interval" : "cron",
      intervalSecs: w.schedule.type === "interval" ? w.schedule.secs : 300,
      cronExpr: w.schedule.type === "cron" ? w.schedule.expr : "0 0 9 * * *",
      command: w.command,
      target_contact: w.target_contact,
      mode: w.mode,
      enabled: w.enabled,
    };
  }

  async function saveDraft() {
    if (!editing) return;
    const schedule = draftToSchedule(editing);
    try {
      if (editing.id) {
        await invoke("ai_bridge_update_watcher", {
          id: editing.id,
          name: editing.name,
          schedule,
          command: editing.command,
          targetContact: editing.target_contact,
          mode: editing.mode,
          enabled: editing.enabled,
        });
      } else {
        await invoke("ai_bridge_add_watcher", {
          name: editing.name,
          schedule,
          command: editing.command,
          targetContact: editing.target_contact,
          mode: editing.mode,
        });
      }
      setEditing(null);
      setCronVerification(null);
      void reload();
    } catch (e) {
      setCronVerification({ ok: false, msg: String(e) });
    }
  }

  async function verifyCron() {
    if (!editing || editing.scheduleType !== "cron") return;
    try {
      const next = await invoke<string>("ai_bridge_validate_cron", {
        expr: editing.cronExpr,
      });
      setCronVerification({
        ok: true,
        msg: t("settings.ai_bridge.watchers.next_fire", { ts: next }),
      });
    } catch (e) {
      setCronVerification({ ok: false, msg: String(e) });
    }
  }

  function clearRowError(id: string) {
    setRowErrors(prev => {
      if (!(id in prev)) return prev;
      const next = { ...prev };
      delete next[id];
      return next;
    });
  }

  async function deleteWatcher(w: Watcher) {
    if (!confirm(t("settings.ai_bridge.watchers.delete_confirm", { name: w.name }))) return;
    clearRowError(w.id);
    try {
      await invoke("ai_bridge_remove_watcher", { id: w.id });
      void reload();
    } catch (e) {
      // Surface inline so the user notices a delete-rejection (file
      // lock, missing id race) instead of having it silently dropped
      // into the dev console.
      setRowErrors(prev => ({
        ...prev,
        [w.id]: t("settings.ai_bridge.watchers.delete_failed", {
          error: String(e),
          defaultValue: "delete failed: {{error}}",
        }),
      }));
    }
  }

  async function toggleEnabled(w: Watcher) {
    clearRowError(w.id);
    try {
      await invoke("ai_bridge_set_watcher_enabled", {
        id: w.id,
        enabled: !w.enabled,
      });
      void reload();
    } catch (e) {
      setRowErrors(prev => ({
        ...prev,
        [w.id]: t("settings.ai_bridge.watchers.toggle_failed", {
          error: String(e),
          defaultValue: "toggle failed: {{error}}",
        }),
      }));
    }
  }

  async function runNow(w: Watcher) {
    // Per-row busy gate. Prior to this, rapid clicks on the "Run now"
    // button could launch one subprocess per click for the same watcher
    // — 10 clicks → 10 concurrent shell runs. The set is keyed by
    // watcher id so different watchers can still run in parallel.
    if (runningSet.has(w.id)) return;
    setRunningSet(prev => {
      const next = new Set(prev);
      next.add(w.id);
      return next;
    });
    try {
      const r = await invoke<WatcherFireResult>("ai_bridge_run_watcher_now", {
        id: w.id,
      });
      setFireResults(prev => ({
        ...prev,
        [w.id]: t("settings.ai_bridge.watchers.fired_just_now", {
          code: r.exit_code,
          sent: r.message_sent ? "yes" : "no",
        }) + ` — ${r.status}`,
      }));
      void reload();
    } catch (e) {
      setFireResults(prev => ({
        ...prev,
        [w.id]: t("settings.ai_bridge.watchers.fire_failed", { error: String(e) }),
      }));
    } finally {
      setRunningSet(prev => {
        const next = new Set(prev);
        next.delete(w.id);
        return next;
      });
    }
  }

  return (
    <div className="border-t border-cyber-cyan/20 pt-3 space-y-2">
      <div className="text-xs text-neon-green">
        {t("settings.ai_bridge.watchers.title")}
      </div>
      <div className="text-[10px] text-soft-grey">
        {t("settings.ai_bridge.watchers.hint")}
      </div>

      {watchers.length === 0 && (
        <div className="text-[10px] text-soft-grey italic">
          {t("settings.ai_bridge.watchers.empty")}
        </div>
      )}

      <div className="space-y-2">
        {watchers.map(w => (
          <div
            key={w.id}
            className="bg-black/40 border border-cyber-cyan/20 px-2 py-2 space-y-1"
          >
            <div className="flex items-center gap-2">
              <span className="font-mono text-xs text-cyber-cyan flex-1">{w.name}</span>
              <label className="flex items-center gap-1 text-[10px] text-soft-grey">
                <input
                  type="checkbox"
                  checked={w.enabled}
                  onChange={() => void toggleEnabled(w)}
                />
                {t("settings.ai_bridge.watchers.enabled_label")}
              </label>
            </div>
            <div className="text-[10px] text-soft-grey font-mono">
              {w.schedule.type === "interval"
                ? `interval ${w.schedule.secs}s`
                : `cron "${w.schedule.expr}"`}
              {" → "}
              <span className="text-neon-magenta">{w.target_contact}</span>
              {" · "}
              <span className="text-neon-green">{w.mode}</span>
            </div>
            <div className="text-[10px] text-soft-grey">
              {t("settings.ai_bridge.watchers.last_run_label")}:{" "}
              {w.last_run_at ?? t("settings.ai_bridge.watchers.never_ran")}
              {w.last_status && (
                <>
                  {" · "}
                  <span
                    className={
                      w.last_status.startsWith("error")
                        ? "text-neon-magenta"
                        : "text-neon-green"
                    }
                  >
                    {w.last_status}
                  </span>
                </>
              )}
            </div>
            {fireResults[w.id] && (
              <div className="text-[10px] text-cyber-cyan italic">
                {fireResults[w.id]}
              </div>
            )}
            {rowErrors[w.id] && (
              <div className="text-[10px] text-neon-magenta">
                ! {rowErrors[w.id]}
              </div>
            )}
            <div className="flex gap-2">
              <button
                onClick={() => void runNow(w)}
                disabled={runningSet.has(w.id)}
                className="neon-button text-[10px] disabled:opacity-40"
              >
                {runningSet.has(w.id)
                  ? t("settings.ai_bridge.watchers.running", {
                      defaultValue: "running…",
                    })
                  : t("settings.ai_bridge.watchers.run_now_button")}
              </button>
              <button
                onClick={() => {
                  setEditing(watcherToDraft(w));
                  setCronVerification(null);
                }}
                className="neon-button text-[10px]"
              >
                {t("settings.ai_bridge.watchers.edit_button")}
              </button>
              <button
                onClick={() => void deleteWatcher(w)}
                className="text-[10px] text-neon-magenta hover:underline"
              >
                {t("settings.ai_bridge.watchers.delete_button")}
              </button>
            </div>
          </div>
        ))}
      </div>

      <button
        onClick={() => {
          setEditing({
            ...EMPTY_DRAFT,
            target_contact: allowlist[0] ?? "",
          });
          setCronVerification(null);
        }}
        className="neon-button text-xs"
      >
        {t("settings.ai_bridge.watchers.add_button")}
      </button>

      {editing && (
        <div className="border border-neon-magenta/40 bg-black/60 p-3 space-y-2 mt-2">
          <div>
            <label className="text-[10px] text-soft-grey block mb-1">
              {t("settings.ai_bridge.watchers.name_label")}
            </label>
            <input
              type="text"
              value={editing.name}
              onChange={e => setEditing({ ...editing, name: e.target.value })}
              className="bg-black border border-cyber-cyan text-cyber-cyan text-xs px-2 py-1 w-full font-mono"
            />
          </div>

          <div>
            <label className="text-[10px] text-soft-grey block mb-1">
              {t("settings.ai_bridge.watchers.schedule_label")}
            </label>
            <div className="flex gap-3 mb-1 text-xs">
              <label className="flex items-center gap-1 text-cyber-cyan">
                <input
                  type="radio"
                  checked={editing.scheduleType === "interval"}
                  onChange={() =>
                    setEditing({ ...editing, scheduleType: "interval" })
                  }
                />
                {t("settings.ai_bridge.watchers.schedule_interval")}
              </label>
              <label className="flex items-center gap-1 text-cyber-cyan">
                <input
                  type="radio"
                  checked={editing.scheduleType === "cron"}
                  onChange={() =>
                    setEditing({ ...editing, scheduleType: "cron" })
                  }
                />
                {t("settings.ai_bridge.watchers.schedule_cron")}
              </label>
            </div>
            {editing.scheduleType === "interval" ? (
              <>
                <input
                  type="number"
                  min={1}
                  value={editing.intervalSecs}
                  onChange={e =>
                    setEditing({
                      ...editing,
                      intervalSecs: parseInt(e.target.value) || 1,
                    })
                  }
                  className="bg-black border border-cyber-cyan text-cyber-cyan text-xs px-2 py-1 w-32 font-mono"
                />
                <div className="text-[10px] text-soft-grey mt-1">
                  {t("settings.ai_bridge.watchers.schedule_interval_hint")}
                </div>
              </>
            ) : (
              <>
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={editing.cronExpr}
                    onChange={e =>
                      setEditing({ ...editing, cronExpr: e.target.value })
                    }
                    className="bg-black border border-cyber-cyan text-cyber-cyan text-xs px-2 py-1 flex-1 font-mono"
                  />
                  <button
                    onClick={() => void verifyCron()}
                    className="neon-button text-[10px]"
                  >
                    {t("settings.ai_bridge.watchers.verify_cron_button")}
                  </button>
                </div>
                <div className="text-[10px] text-soft-grey mt-1">
                  {t("settings.ai_bridge.watchers.schedule_cron_hint")}
                </div>
                {cronVerification && (
                  <div
                    className={`text-[10px] mt-1 ${
                      cronVerification.ok
                        ? "text-neon-green"
                        : "text-neon-magenta"
                    }`}
                  >
                    {cronVerification.msg}
                  </div>
                )}
              </>
            )}
          </div>

          <div>
            <label className="text-[10px] text-soft-grey block mb-1">
              {t("settings.ai_bridge.watchers.command_label")}
            </label>
            <textarea
              value={editing.command}
              onChange={e =>
                setEditing({ ...editing, command: e.target.value })
              }
              rows={3}
              placeholder={t("settings.ai_bridge.watchers.command_placeholder")}
              className="bg-black border border-cyber-cyan text-cyber-cyan text-xs px-2 py-1 w-full font-mono"
            />
          </div>

          <div>
            <label className="text-[10px] text-soft-grey block mb-1">
              {t("settings.ai_bridge.watchers.target_label")}
            </label>
            {allowlist.length === 0 ? (
              <div className="text-[10px] text-neon-magenta italic">
                {t("settings.ai_bridge.watchers.target_none")}
              </div>
            ) : (
              <select
                value={editing.target_contact}
                onChange={e =>
                  setEditing({ ...editing, target_contact: e.target.value })
                }
                className="bg-black border border-cyber-cyan text-cyber-cyan text-xs px-2 py-1 w-full font-mono"
              >
                {allowlist.map(label => (
                  <option key={label} value={label}>
                    {label}
                  </option>
                ))}
              </select>
            )}
          </div>

          <div>
            <label className="text-[10px] text-soft-grey block mb-1">
              {t("settings.ai_bridge.watchers.mode_label")}
            </label>
            <div className="space-y-1 text-xs text-cyber-cyan">
              {(["raw", "summarize", "alert_only"] as const).map(m => (
                <label key={m} className="flex items-center gap-1">
                  <input
                    type="radio"
                    checked={editing.mode === m}
                    onChange={() => setEditing({ ...editing, mode: m })}
                  />
                  {t(`settings.ai_bridge.watchers.mode_${m}`)}
                </label>
              ))}
            </div>
          </div>

          <div className="flex gap-2 pt-1">
            <button
              onClick={() => void saveDraft()}
              disabled={
                !editing.name ||
                !editing.command ||
                !editing.target_contact ||
                allowlist.length === 0
              }
              className="neon-button text-xs disabled:opacity-40"
            >
              {t("settings.ai_bridge.watchers.save_button")}
            </button>
            <button
              onClick={() => {
                setEditing(null);
                setCronVerification(null);
              }}
              className="text-xs text-soft-grey hover:underline"
            >
              {t("settings.ai_bridge.watchers.cancel_button")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
