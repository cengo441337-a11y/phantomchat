//! Wave 11E — Proactive Watchers.
//!
//! A "Watcher" is a cron-like trigger that runs a shell command on a schedule
//! and pushes the result (raw, summarised, or alert-only) to a pre-configured
//! contact via the existing AI-Bridge send pipeline. Unlike the reactive
//! `ai_bridge_maybe_handle` path which only fires on incoming messages, a
//! watcher pushes UNPROMPTED.
//!
//! ## Use cases
//!
//! - "Watch CI for green build, message me when it passes."
//! - "Every 30 minutes, run my deploy-status check and push any changes."
//! - "When the build breaks, ping me." (`AlertOnly` mode)
//! - "Every morning at 9 AM, summarise overnight logs." (`Summarize` mode)
//!
//! ## Threat model
//!
//! Watchers run shell commands AS THE BRIDGE PROCESS USER. Anyone who can
//! edit `ai_bridge_watchers.json` can execute arbitrary code on the home
//! machine — same risk class as the existing `claude_cli_skip_permissions`
//! flag. Both are gated by the OS-level "who can write to `<app_data>`".
//! Every add/remove/update + every fire is audit-logged via the standard
//! `ai_bridge.watcher_*` audit categories.
//!
//! ## Scheduling
//!
//! A single tokio task ticks once per second, scans every enabled watcher,
//! and fires each one whose schedule has come due:
//!
//! - `Interval { secs: N }` fires when `now - last_run_at >= N`.
//! - `Cron { expr }` fires when `now >= cron::Schedule::upcoming(Local).next()`
//!   computed from the *previous* run timestamp (or process-start if never
//!   run).
//!
//! There is **no per-watcher concurrency lock** in this iteration — if a
//! watcher's command takes longer than its schedule interval, the next
//! tick will spawn a second invocation in parallel. Punted to Wave 11G if
//! anyone hits it; the obvious fix is a `tokio::sync::Mutex` per watcher
//! id stashed in a process-global `HashMap<String, Arc<Mutex<()>>>`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::process::Command as TokioCommand;
use tokio::sync::Mutex as AsyncMutex;

use crate::ai_bridge;

const WATCHERS_FILE: &str = "ai_bridge_watchers.json";
/// Cap watcher stdout payloads at this many chars before piping into the
/// LLM or sending raw. Defends against a runaway command flooding the
/// chat with megabytes of output.
const MAX_STDOUT_CHARS: usize = 8000;
/// Hard wall-clock timeout per command invocation. Prevents a hung
/// process from holding a watcher slot forever.
const COMMAND_TIMEOUT_SECS: u64 = 300;
/// LLM system prompt for `WatcherMode::Summarize`. Kept terse so cheap
/// models stay on-task; the watcher fires every 30 s in some configs and
/// the user does not want a wall of text per push.
const SUMMARIZE_SYSTEM_PROMPT: &str =
    "Summarize this in 1-3 sentences. Only mention notable / actionable items.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum WatcherSchedule {
    /// Fire every N seconds. Internal scheduler tracks `last_run_at` and
    /// fires once `now - last_run_at >= secs`.
    Interval { secs: u64 },
    /// Standard cron expression — uses the `cron` crate which expects 6 or
    /// 7 fields (sec min hour dom mon dow [year]).
    Cron { expr: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WatcherMode {
    /// Send the command's stdout verbatim, prefixed with "🔔 [<name>]\n".
    Raw,
    /// Pipe stdout through the LLM with a brief summarise system prompt.
    Summarize,
    /// Only send if the exit code is non-zero. Useful for "ping me iff CI
    /// fails", health checks, and other "no news is good news" scenarios.
    AlertOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watcher {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub schedule: WatcherSchedule,
    pub command: String,
    pub target_contact: String,
    pub mode: WatcherMode,
    /// ISO 8601 (UTC) timestamp of the last fire. `None` until the first
    /// run completes — the scheduler treats `None` as "fire on next tick"
    /// for `Interval` schedules and "fire on next cron match after process
    /// start" for `Cron` schedules.
    #[serde(default)]
    pub last_run_at: Option<String>,
    /// Short status line — `"ok"`, `"error: <reason>"`, `"alert: exit <code>"`.
    /// Surfaces in the Settings UI so the user can spot a misbehaving
    /// watcher without opening the audit log.
    #[serde(default)]
    pub last_status: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WatchersDisk {
    watchers: Vec<Watcher>,
}

pub fn watchers_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(WATCHERS_FILE)
}

pub fn load_watchers(app_data_dir: &Path) -> Vec<Watcher> {
    std::fs::read(watchers_path(app_data_dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<WatchersDisk>(&b).ok())
        .map(|d| d.watchers)
        .unwrap_or_default()
}

pub fn save_watchers(app_data_dir: &Path, watchers: &[Watcher]) -> Result<()> {
    let path = watchers_path(app_data_dir);
    let disk = WatchersDisk {
        watchers: watchers.to_vec(),
    };
    let buf = serde_json::to_vec_pretty(&disk)
        .with_context(|| "serialising watchers")?;
    std::fs::write(&path, buf)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Generate a v4 UUID (RFC 4122) using `rand::OsRng`. Hand-rolled rather
/// than pulling in the `uuid` crate just for this — the format is
/// fixed and the crate adds ~30 KB to the bundle for one function.
pub fn new_watcher_id() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    // Version 4 — top 4 bits of byte 6.
    buf[6] = (buf[6] & 0x0f) | 0x40;
    // Variant — top 2 bits of byte 8.
    buf[8] = (buf[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        buf[0], buf[1], buf[2], buf[3],
        buf[4], buf[5],
        buf[6], buf[7],
        buf[8], buf[9],
        buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
    )
}

/// Validate a cron expression by attempting to parse + compute the next
/// fire time. Returns the next-fire ISO 8601 timestamp on success so the
/// UI can show "next fire: 2026-04-26T09:00:00+02:00".
pub fn validate_cron_expression(expr: &str) -> Result<String> {
    use std::str::FromStr;
    let schedule = cron::Schedule::from_str(expr)
        .map_err(|e| anyhow!("cron parse error: {}", e))?;
    let next = schedule
        .upcoming(Local)
        .next()
        .ok_or_else(|| anyhow!("cron expression yields no future fires"))?;
    Ok(next.to_rfc3339())
}

pub fn add_watcher(
    app_data_dir: &Path,
    name: String,
    schedule: WatcherSchedule,
    command: String,
    target_contact: String,
    mode: WatcherMode,
) -> Result<Watcher> {
    // Validate cron expressions up front so the user gets the error in the
    // add flow instead of silently failing on the next tick.
    if let WatcherSchedule::Cron { expr } = &schedule {
        validate_cron_expression(expr)
            .with_context(|| format!("invalid cron expression '{}'", expr))?;
    }
    let watcher = Watcher {
        id: new_watcher_id(),
        name,
        enabled: true,
        schedule,
        command,
        target_contact,
        mode,
        last_run_at: None,
        last_status: None,
    };
    let mut all = load_watchers(app_data_dir);
    all.push(watcher.clone());
    save_watchers(app_data_dir, &all)?;
    Ok(watcher)
}

pub fn remove_watcher(app_data_dir: &Path, id: &str) -> Result<bool> {
    let mut all = load_watchers(app_data_dir);
    let before = all.len();
    all.retain(|w| w.id != id);
    if all.len() == before {
        return Ok(false);
    }
    save_watchers(app_data_dir, &all)?;
    Ok(true)
}

/// Update the mutable fields of a watcher. `id` is immutable and used as
/// the lookup key; everything else is replaced wholesale. Returns the
/// updated record so the caller can echo it back to the UI.
pub fn update_watcher(
    app_data_dir: &Path,
    id: &str,
    name: String,
    schedule: WatcherSchedule,
    command: String,
    target_contact: String,
    mode: WatcherMode,
    enabled: bool,
) -> Result<Watcher> {
    if let WatcherSchedule::Cron { expr } = &schedule {
        validate_cron_expression(expr)
            .with_context(|| format!("invalid cron expression '{}'", expr))?;
    }
    let mut all = load_watchers(app_data_dir);
    let entry = all
        .iter_mut()
        .find(|w| w.id == id)
        .ok_or_else(|| anyhow!("no watcher with id '{}'", id))?;
    entry.name = name;
    entry.schedule = schedule;
    entry.command = command;
    entry.target_contact = target_contact;
    entry.mode = mode;
    entry.enabled = enabled;
    let updated = entry.clone();
    save_watchers(app_data_dir, &all)?;
    Ok(updated)
}

pub fn set_watcher_enabled(app_data_dir: &Path, id: &str, enabled: bool) -> Result<()> {
    let mut all = load_watchers(app_data_dir);
    let entry = all
        .iter_mut()
        .find(|w| w.id == id)
        .ok_or_else(|| anyhow!("no watcher with id '{}'", id))?;
    entry.enabled = enabled;
    save_watchers(app_data_dir, &all)
}

/// Persist the post-fire bookkeeping (timestamp + status string) for a
/// single watcher. Unlike `update_watcher` this preserves the user-edited
/// fields — the scheduler only rewrites `last_run_at` and `last_status`.
fn record_run(
    app_data_dir: &Path,
    id: &str,
    last_run_at: String,
    last_status: String,
) -> Result<()> {
    let mut all = load_watchers(app_data_dir);
    if let Some(entry) = all.iter_mut().find(|w| w.id == id) {
        entry.last_run_at = Some(last_run_at);
        entry.last_status = Some(last_status);
        save_watchers(app_data_dir, &all)?;
    }
    Ok(())
}

/// Run a single watcher: spawn its command, gather stdout + exit, apply
/// the `WatcherMode` policy, send via `send_message_inner`, and return
/// the post-run summary so the Tauri command can echo it to the UI.
///
/// The caller is responsible for emitting audit-log entries — this
/// function is pure I/O so it can be used both by the scheduler tick and
/// by the manual-fire Tauri command.
pub struct WatcherRunOutcome {
    pub stdout: String,
    pub exit_code: i32,
    pub message_sent: bool,
    /// Human-friendly status string, persisted into `last_status`. One of:
    ///   - `"ok"`            — sent successfully
    ///   - `"alert: exit <c>"` — AlertOnly mode, exit nonzero, sent
    ///   - `"skipped: ok"`   — AlertOnly mode, exit zero, no send
    ///   - `"error: <reason>"` — anything went wrong
    pub status: String,
}

pub async fn run_watcher_once(
    app: &AppHandle,
    watcher: &Watcher,
    send_fn: SendFn,
) -> WatcherRunOutcome {
    // Spawn under sh / cmd to get shell semantics (pipes, env-var
    // expansion, etc.). The command field is treated as a single shell
    // string so the user can paste anything they'd type into a terminal.
    #[cfg(target_family = "unix")]
    let mut cmd = {
        let mut c = TokioCommand::new("sh");
        c.arg("-c").arg(&watcher.command);
        c
    };
    #[cfg(target_family = "windows")]
    let mut cmd = {
        let mut c = TokioCommand::new("cmd");
        c.arg("/c").arg(&watcher.command);
        c
    };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return WatcherRunOutcome {
                stdout: String::new(),
                exit_code: -1,
                message_sent: false,
                status: format!("error: spawn failed: {}", e),
            };
        }
    };

    let output = match tokio::time::timeout(
        Duration::from_secs(COMMAND_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return WatcherRunOutcome {
                stdout: String::new(),
                exit_code: -1,
                message_sent: false,
                status: format!("error: wait failed: {}", e),
            };
        }
        Err(_) => {
            return WatcherRunOutcome {
                stdout: String::new(),
                exit_code: -1,
                message_sent: false,
                status: format!("error: command timed out after {}s", COMMAND_TIMEOUT_SECS),
            };
        }
    };

    let exit_code = output.status.code().unwrap_or(-1);
    let mut stdout_text = String::from_utf8_lossy(&output.stdout).to_string();
    if stdout_text.chars().count() > MAX_STDOUT_CHARS {
        stdout_text = stdout_text
            .chars()
            .take(MAX_STDOUT_CHARS)
            .collect::<String>()
            + "\n…[truncated]";
    }

    // Defensive: if the target contact isn't on the bridge allowlist, do
    // not send. Prevents a mistyped `target_contact` from leaking to a
    // wrong recipient (or a never-existed label that would fail at
    // send-time anyway).
    let app_data_dir = match crate::app_data_dir_for(app) {
        Ok(d) => d,
        Err(_) => {
            return WatcherRunOutcome {
                stdout: stdout_text,
                exit_code,
                message_sent: false,
                status: "error: cannot resolve app_data_dir".to_string(),
            };
        }
    };
    let cfg = ai_bridge::load_config(&app_data_dir);
    let on_allowlist = cfg
        .allowlist
        .iter()
        .any(|l| l == &watcher.target_contact);
    if !on_allowlist {
        return WatcherRunOutcome {
            stdout: stdout_text,
            exit_code,
            message_sent: false,
            status: format!(
                "error: target '{}' not on AI-bridge allowlist",
                watcher.target_contact
            ),
        };
    }

    // Apply the mode policy to decide what (if anything) to send.
    let body_to_send: Option<String> = match watcher.mode {
        WatcherMode::Raw => Some(format!("🔔 [{}]\n{}", watcher.name, stdout_text.trim_end())),
        WatcherMode::Summarize => {
            // Borrow the bridge's existing system_prompt config but
            // override the system prompt for this one call. We do this
            // by cloning + mutating instead of plumbing a per-call arg
            // through `complete()` so the public API stays narrow.
            let mut summarize_cfg = cfg.clone();
            summarize_cfg.system_prompt = SUMMARIZE_SYSTEM_PROMPT.to_string();
            match ai_bridge::complete(&summarize_cfg, &[], &stdout_text).await {
                Ok(summary) => Some(format!(
                    "🔔 [{}]\n{}",
                    watcher.name,
                    summary.trim_end()
                )),
                Err(e) => {
                    return WatcherRunOutcome {
                        stdout: stdout_text,
                        exit_code,
                        message_sent: false,
                        status: format!("error: summarize failed: {}", e),
                    };
                }
            }
        }
        WatcherMode::AlertOnly => {
            if exit_code == 0 {
                None
            } else {
                Some(format!(
                    "🔔 [{}] exit={} \n{}",
                    watcher.name, exit_code, stdout_text.trim_end()
                ))
            }
        }
    };

    let message_sent = match body_to_send {
        Some(body) => match send_fn(app.clone(), watcher.target_contact.clone(), body).await {
            Ok(()) => true,
            Err(e) => {
                return WatcherRunOutcome {
                    stdout: stdout_text,
                    exit_code,
                    message_sent: false,
                    status: format!("error: send failed: {}", e),
                };
            }
        },
        None => {
            // Drop the unused closure so SendFn's FnOnce contract is
            // satisfied even on the no-send branch.
            drop(send_fn);
            false
        }
    };

    let status = match watcher.mode {
        WatcherMode::AlertOnly if !message_sent => "skipped: ok".to_string(),
        WatcherMode::AlertOnly => format!("alert: exit {}", exit_code),
        _ => "ok".to_string(),
    };

    WatcherRunOutcome {
        stdout: stdout_text,
        exit_code,
        message_sent,
        status,
    }
}

// Helper aliases so `run_watcher_once` can accept any future-returning send
// closure (lib.rs holds the actual `send_message_inner`). We box the future
// so the function signature stays object-safe for the scheduler's
// `Arc<dyn Fn>` re-use across ticks.
pub type SendFut = std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>;
pub type SendFn = Box<dyn FnOnce(AppHandle, String, String) -> SendFut + Send>;

/// Persist post-run bookkeeping + emit an audit log + a status event.
/// Called by both the scheduler and the manual-fire Tauri command.
pub fn record_run_and_audit(
    app: &AppHandle,
    app_data_dir: &Path,
    watcher: &Watcher,
    outcome: &WatcherRunOutcome,
) {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let _ = record_run(app_data_dir, &watcher.id, now, outcome.status.clone());
    let category = "ai_bridge";
    let event = if outcome.status.starts_with("error") {
        "watcher_failed"
    } else {
        "watcher_fired"
    };
    crate::audit_for_watchers(
        app,
        category,
        event,
        serde_json::json!({
            "id": watcher.id,
            "name": watcher.name,
            "target": watcher.target_contact,
            "mode": watcher.mode,
            "exit_code": outcome.exit_code,
            "message_sent": outcome.message_sent,
            "status": outcome.status,
        }),
    );
    // Also push a frontend event so the Settings list updates without a
    // manual reload.
    let _ = app.emit("ai_bridge_watcher_fired", &watcher.id);
}

/// Decide whether a watcher is due to fire right now. Returns true iff
/// the schedule has elapsed since the persisted `last_run_at`.
fn watcher_is_due(watcher: &Watcher, now: DateTime<Utc>) -> bool {
    match &watcher.schedule {
        WatcherSchedule::Interval { secs } => {
            let last = match watcher
                .last_run_at
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
            {
                Some(t) => t,
                // Never run before — fire immediately so the first watcher
                // tick is responsive instead of waiting `secs` first.
                None => return true,
            };
            let elapsed = (now - last).num_seconds().max(0) as u64;
            elapsed >= *secs
        }
        WatcherSchedule::Cron { expr } => {
            use std::str::FromStr;
            let Ok(schedule) = cron::Schedule::from_str(expr) else {
                return false;
            };
            // Anchor the lookup at `last_run_at` if we have one (so we
            // don't double-fire after a rapid restart) or the process
            // boot time approximated via `now - 1s`.
            let anchor: DateTime<Local> = watcher
                .last_run_at
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Local))
                .unwrap_or_else(|| (now - chrono::Duration::seconds(1)).with_timezone(&Local));
            match schedule.after(&anchor).next() {
                Some(next_fire) => next_fire.with_timezone(&Utc) <= now,
                None => false,
            }
        }
    }
}

/// Spawn the long-running scheduler task. Wakes once per second, scans
/// all enabled watchers, fires each due one in its own task. Idempotent
/// via a process-global atomic — calling twice from `setup()` is a no-op.
///
/// The fire-task uses a `send_message_inner` shim provided by `lib.rs` so
/// this module doesn't need to know about the cargo-feature-gated bits of
/// the send pipeline.
/// Type of the send-function dispatcher passed in from `lib.rs`. A plain
/// `Arc<Fn>` rather than a generic so the scheduler closure stays
/// object-safe across the spawned-task boundary.
pub type SendDispatcher = Arc<dyn Fn(AppHandle, String, String) -> SendFut + Send + Sync>;

pub fn start_scheduler(app: AppHandle, send_message_inner: SendDispatcher) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    // Coarse global "is anything running" mutex so a single watcher with a
    // very long-running command can't completely starve other watchers'
    // tick visibility — fires happen serially within a tick, but the tick
    // loop itself stays responsive.
    let in_flight: Arc<AsyncMutex<()>> = Arc::new(AsyncMutex::new(()));

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        // First tick fires immediately — harmless, just causes one
        // extra schedule scan a few ms after spawn.
        loop {
            interval.tick().await;
            let app_data_dir = match crate::app_data_dir_for(&app) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let watchers = load_watchers(&app_data_dir);
            let now = chrono::Utc::now();
            for w in watchers.into_iter().filter(|w| w.enabled) {
                if !watcher_is_due(&w, now) {
                    continue;
                }
                let app_inner = app.clone();
                let app_data_inner = app_data_dir.clone();
                let send_inner = send_message_inner.clone();
                let lock = in_flight.clone();
                tokio::spawn(async move {
                    let _guard = lock.lock().await;
                    let send_box: SendFn = Box::new(move |a, l, b| send_inner(a, l, b));
                    let outcome = run_watcher_once(&app_inner, &w, send_box).await;
                    record_run_and_audit(&app_inner, &app_data_inner, &w, &outcome);
                });
            }
        }
    });
}

/// Dispatched from the manual-fire Tauri command. Public surface returned
/// to the frontend mirrors the Tauri-command spec ({stdout, exit_code,
/// message_sent}); the audit-log + last_status side effects are handled
/// inline so the UI stays consistent with the scheduler path.
pub async fn manual_fire(
    app: &AppHandle,
    id: &str,
    send_message_inner: SendFn,
) -> Result<WatcherRunOutcome> {
    let app_data_dir = crate::app_data_dir_for(app)?;
    let all = load_watchers(&app_data_dir);
    let watcher = all
        .into_iter()
        .find(|w| w.id == id)
        .ok_or_else(|| anyhow!("no watcher with id '{}'", id))?;
    let outcome = run_watcher_once(app, &watcher, send_message_inner).await;
    record_run_and_audit(app, &app_data_dir, &watcher, &outcome);
    Ok(outcome)
}

/// Keep the runtime list keyed by id so add/remove from the UI doesn't
/// require a full process restart. Reads the on-disk file fresh — the
/// scheduler task does the same on every tick so additions show up
/// within ~1 s without any cross-task plumbing.
pub fn list_watchers(app_data_dir: &Path) -> Vec<Watcher> {
    load_watchers(app_data_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_validation_accepts_basic_expressions() {
        // Hourly at 0 seconds, 0 minutes
        assert!(validate_cron_expression("0 0 * * * *").is_ok());
        // Every Monday at 09:00
        assert!(validate_cron_expression("0 0 9 * * Mon").is_ok());
        // Every minute
        assert!(validate_cron_expression("0 * * * * *").is_ok());
    }

    #[test]
    fn cron_validation_rejects_garbage() {
        assert!(validate_cron_expression("not a cron").is_err());
        assert!(validate_cron_expression("").is_err());
    }

    #[test]
    fn uuid_v4_format() {
        let id = new_watcher_id();
        assert_eq!(id.len(), 36);
        // Position 14 == version nibble, must be '4'
        assert_eq!(&id[14..15], "4");
        // Position 19 == variant nibble, must be 8|9|a|b
        let v = &id[19..20];
        assert!(matches!(v, "8" | "9" | "a" | "b"));
    }

    #[test]
    fn interval_due_fires_immediately_first_time() {
        let w = Watcher {
            id: "x".into(),
            name: "test".into(),
            enabled: true,
            schedule: WatcherSchedule::Interval { secs: 60 },
            command: "echo hi".into(),
            target_contact: "alice".into(),
            mode: WatcherMode::Raw,
            last_run_at: None,
            last_status: None,
        };
        assert!(watcher_is_due(&w, Utc::now()));
    }

    #[test]
    fn interval_due_respects_last_run() {
        let w = Watcher {
            id: "x".into(),
            name: "test".into(),
            enabled: true,
            schedule: WatcherSchedule::Interval { secs: 600 },
            command: "echo hi".into(),
            target_contact: "alice".into(),
            mode: WatcherMode::Raw,
            last_run_at: Some(chrono::Utc::now().to_rfc3339()),
            last_status: None,
        };
        assert!(!watcher_is_due(&w, Utc::now()));
    }
}

