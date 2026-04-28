//! `phantom-build-org-msi` — PhantomChat enterprise MSI templater.
//!
//! Takes an org-name + members CSV + relays text file + a base MSI and
//! produces a deployment artifact directory containing:
//!
//!   * `bootstrap.json` — the per-org pre-seed file the desktop app reads
//!     on first launch (see `desktop/src-tauri/src/lib.rs::try_apply_bootstrap`).
//!   * `deploy.ps1`     — Windows PowerShell installer that runs the base
//!     MSI in silent mode and then drops `bootstrap.json` into the
//!     per-user `%APPDATA%\de.dc-infosec.phantomchat\` directory so the
//!     next app launch detects + applies it.
//!   * `README.txt`     — short admin-facing deployment recipe.
//!
//! ## Why a fallback artifact instead of true MSI re-bundling?
//!
//! True MSI re-bundling (re-packing the WiX-generated CAB + updating the
//! MSI tables) is Windows + WiX-toolset-only territory. We're a Linux-side
//! tool — and even on Windows the WiX SDK round-trip would force every
//! contributor onto a single Microsoft toolchain. The deploy.ps1 fallback
//! achieves the same B2B outcome (one IT-Admin push → all PCs auto-enrolled
//! against the org directory) without coupling the templater to WiX.
//!
//! Wave 7C-followup may revisit true MSI re-bundling once the WiX heat
//! command is wrapped in a cross-platform Rust shim. For now the script-
//! based path is documented + tested + good enough for SCCM / Intune /
//! Group-Policy Software-Installation.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use clap::Parser;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

// ── Bootstrap schema ─────────────────────────────────────────────────────────
//
// This MUST stay in lock-step with the `BootstrapFile` reader in
// `desktop/src-tauri/src/lib.rs`. `schema_version` is the explicit forward-
// compat lever: bump it on any incompatible field change so the desktop
// reader can refuse + log instead of silently mis-applying old data.

const BOOTSTRAP_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct BootstrapFile {
    schema_version: u32,
    org_name: String,
    org_id: String,
    /// 32-byte CSPRNG, base64-std-encoded. Reserved for future use (org-
    /// internal directory-update signing). Persisted by the desktop app
    /// but not yet wired into any send/receive path.
    org_secret: String,
    default_relays: Vec<String>,
    directory: Vec<DirectoryEntry>,
    /// 7-char Wave-7A mDNS join code. Optional — when present, the
    /// desktop app spawns `lan_org_join(code)` after bootstrap apply so
    /// the install auto-discovers other org members on the office LAN.
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_join_lan_org_code: Option<String>,
    branding: Branding,
}

#[derive(Debug, Serialize, Deserialize)]
struct DirectoryEntry {
    label: String,
    address: String,
    signing_pub_hex: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Branding {
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    primary_color: Option<String>,
}

// ── Members CSV row ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct MemberCsvRow {
    label: String,
    address: String,
    signing_pub_hex: String,
}

// ── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(
    name = "phantom-build-org-msi",
    about = "Bake a per-org bootstrap into a PhantomChat enterprise deploy artifact.",
    version
)]
struct Cli {
    /// Human-readable organization name, e.g. "Kanzlei Schmidt & Partner".
    #[arg(long)]
    org_name: String,

    /// Stable machine-readable org identifier, e.g. "kanzlei-schmidt-2026-04".
    /// Used as the audit-log discriminator and the `org_id` baked into
    /// every install.
    #[arg(long)]
    org_id: String,

    /// Path to a members CSV with header `label,address,signing_pub_hex`.
    #[arg(long)]
    members: PathBuf,

    /// Path to a UTF-8 text file with one `wss://...` relay URL per line.
    /// Blank lines and `#`-prefixed comments are ignored.
    #[arg(long)]
    relays: PathBuf,

    /// Path to the upstream `PhantomChat_<ver>_x64_en-US.msi` produced by
    /// `tauri build`. The templater never opens this file — it's only
    /// referenced by the generated deploy.ps1 so the admin doesn't have
    /// to know the canonical MSI filename.
    #[arg(long)]
    base_msi: PathBuf,

    /// Output artifact path. May end in `.msi` (kept for CLI ergonomics
    /// even though we emit a directory of artifacts next to it). The tool
    /// writes:
    ///
    /// * `<out>.bootstrap.json`
    /// * `<out>.deploy.ps1`
    /// * `<out>.README.txt`
    ///
    /// alongside (i.e. in the same parent dir as) `<out>` itself.
    #[arg(long)]
    out: PathBuf,

    /// Optional branding display name (window title / tray tooltip).
    #[arg(long)]
    branding_display_name: Option<String>,

    /// Optional branding primary color, e.g. `#00FF9F`.
    #[arg(long)]
    branding_primary_color: Option<String>,

    /// Skip generating an `auto_join_lan_org_code`. Use when the org
    /// has no office LAN (pure remote workforce) — the field is omitted
    /// from the bootstrap so the desktop app skips the mDNS auto-join.
    #[arg(long, default_value_t = false)]
    no_lan_code: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli)
}

fn run(cli: Cli) -> Result<()> {
    // 1. Load + validate inputs ---------------------------------------------
    let members = read_members(&cli.members)
        .with_context(|| format!("reading members CSV {}", cli.members.display()))?;
    if members.is_empty() {
        return Err(anyhow!(
            "members CSV {} is empty — refusing to build a directory-less org bootstrap",
            cli.members.display()
        ));
    }
    let relays = read_relays(&cli.relays)
        .with_context(|| format!("reading relays file {}", cli.relays.display()))?;
    if relays.is_empty() {
        return Err(anyhow!(
            "relays file {} contained no usable URLs",
            cli.relays.display()
        ));
    }
    if !cli.base_msi.exists() {
        // Best-effort warn — the templater never reads the MSI itself, but
        // a stale path here means deploy.ps1 will fail at run-time on the
        // admin's box. Catch it now while the human is still watching.
        eprintln!(
            "warning: --base-msi path {} does not exist locally; deploy.ps1 will reference it as-is",
            cli.base_msi.display()
        );
    }

    // 2. Generate per-org secrets -------------------------------------------
    let org_secret_b64 = generate_org_secret();
    let lan_code = if cli.no_lan_code {
        None
    } else {
        Some(generate_lan_code())
    };

    // 3. Build bootstrap struct ---------------------------------------------
    let bootstrap = BootstrapFile {
        schema_version: BOOTSTRAP_SCHEMA_VERSION,
        org_name: cli.org_name.clone(),
        org_id: cli.org_id.clone(),
        org_secret: org_secret_b64,
        default_relays: relays,
        directory: members
            .into_iter()
            .map(|m| DirectoryEntry {
                label: m.label,
                address: m.address,
                signing_pub_hex: m.signing_pub_hex,
            })
            .collect(),
        auto_join_lan_org_code: lan_code.clone(),
        branding: Branding {
            display_name: cli.branding_display_name.clone(),
            primary_color: cli.branding_primary_color.clone(),
        },
    };

    // 4. Emit artifacts ------------------------------------------------------
    let out_parent = cli
        .out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&out_parent)
        .with_context(|| format!("mkdir -p {}", out_parent.display()))?;

    let stem = cli
        .out
        .file_name()
        .ok_or_else(|| anyhow!("--out has no filename component"))?
        .to_string_lossy()
        .to_string();

    let bootstrap_path = out_parent.join(format!("{}.bootstrap.json", stem));
    let deploy_ps1_path = out_parent.join(format!("{}.deploy.ps1", stem));
    let readme_path = out_parent.join(format!("{}.README.txt", stem));

    let bootstrap_json = serde_json::to_string_pretty(&bootstrap)
        .context("serializing bootstrap.json")?;
    fs::write(&bootstrap_path, &bootstrap_json)
        .with_context(|| format!("writing {}", bootstrap_path.display()))?;

    let ps1 = render_deploy_ps1(&cli.base_msi, &bootstrap_path, &cli.org_id);
    fs::write(&deploy_ps1_path, ps1)
        .with_context(|| format!("writing {}", deploy_ps1_path.display()))?;

    let readme = render_readme(
        &cli.org_name,
        &cli.org_id,
        &bootstrap_path,
        &deploy_ps1_path,
        &cli.base_msi,
        lan_code.as_deref(),
    );
    fs::write(&readme_path, readme)
        .with_context(|| format!("writing {}", readme_path.display()))?;

    println!("phantom-build-org-msi: artifacts emitted");
    println!("  bootstrap : {}", bootstrap_path.display());
    println!("  deploy.ps1: {}", deploy_ps1_path.display());
    println!("  README    : {}", readme_path.display());
    if let Some(code) = lan_code.as_deref() {
        println!("  lan code  : {}", code);
    }
    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn read_members(path: &Path) -> Result<Vec<MemberCsvRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_path(path)?;
    let mut out = Vec::new();
    for (idx, rec) in rdr.deserialize::<MemberCsvRow>().enumerate() {
        let row = rec.with_context(|| format!("members CSV row {} parse failed", idx + 2))?;
        if row.label.is_empty() || row.address.is_empty() || row.signing_pub_hex.is_empty() {
            return Err(anyhow!(
                "members CSV row {} has an empty field — all of label/address/signing_pub_hex are required",
                idx + 2
            ));
        }
        out.push(row);
    }
    Ok(out)
}

fn read_relays(path: &Path) -> Result<Vec<String>> {
    let raw = fs::read_to_string(path)?;
    let urls: Vec<String> = raw
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();
    for u in &urls {
        if !(u.starts_with("ws://") || u.starts_with("wss://")) {
            return Err(anyhow!(
                "relays file contains non-websocket URL {:?} — must start with ws:// or wss://",
                u
            ));
        }
    }
    Ok(urls)
}

/// 32 CSPRNG bytes, base64-std-encoded. Reserved for org-internal directory-
/// update signing in a follow-up wave.
fn generate_org_secret() -> String {
    let mut buf = [0u8; 32];
    OsRng.fill_bytes(&mut buf);
    B64.encode(buf)
}

/// 7-char Wave-7A mDNS join code. Format `XXX-NNNN` where the alphabet is
/// upper-case A-Z + 0-9 minus visually-ambiguous chars (`0`/`O`, `1`/`I`).
/// Short on purpose — admins read these aloud to onsite staff.
fn generate_lan_code() -> String {
    const ALPHA: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut buf = [0u8; 7];
    OsRng.fill_bytes(&mut buf);
    let mut s = String::with_capacity(8);
    for (i, b) in buf.iter().enumerate() {
        if i == 3 {
            s.push('-');
        }
        s.push(ALPHA[(*b as usize) % ALPHA.len()] as char);
    }
    s
}

fn render_deploy_ps1(base_msi: &Path, bootstrap_path: &Path, org_id: &str) -> String {
    let bootstrap_filename = bootstrap_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "bootstrap.json".to_string());
    let base_msi_filename = base_msi
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "PhantomChat.msi".to_string());

    format!(
        r#"# PhantomChat enterprise deploy script — generated by phantom-build-org-msi.
# Org: {org_id}
#
# Run on each target PC (SCCM / Intune / Group Policy / scheduled task).
# Requires the script's directory to also contain:
#   * {base_msi_filename}        (the upstream PhantomChat MSI)
#   * {bootstrap_filename}       (the per-org bootstrap blob, baked above)
#
# The script is idempotent — re-running it on an already-installed host
# refreshes the bootstrap.json and lets the next PhantomChat launch re-apply
# the org directory.

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

$msi  = Join-Path $here "{base_msi_filename}"
$boot = Join-Path $here "{bootstrap_filename}"

if (-not (Test-Path $msi))  {{ throw "missing $msi" }}
if (-not (Test-Path $boot)) {{ throw "missing $boot" }}

# 1. Silent install of the upstream MSI -------------------------------------
Write-Host "[phantomchat-deploy] installing $msi ..."
$proc = Start-Process -FilePath "msiexec.exe" `
    -ArgumentList @("/i", "`"$msi`"", "/qn", "/norestart") `
    -Wait -PassThru
if ($proc.ExitCode -ne 0 -and $proc.ExitCode -ne 3010) {{
    throw "msiexec failed with exit code $($proc.ExitCode)"
}}

# 2. Drop bootstrap.json into the per-user app-data dir ---------------------
# PhantomChat looks in two locations on first launch:
#   a. %APPDATA%\de.dc-infosec.phantomchat\bootstrap.json   <- this script
#   b. <install_dir>\bootstrap.json                         <- MSI re-bundle
# We use (a) because it works regardless of how the MSI was packed, and
# crucially survives MSI upgrades (per-user data isn't touched by msiexec).

$appData = Join-Path $env:APPDATA "de.dc-infosec.phantomchat"
New-Item -ItemType Directory -Force -Path $appData | Out-Null
Copy-Item -Force $boot (Join-Path $appData "bootstrap.json")

Write-Host "[phantomchat-deploy] bootstrap.json staged at $appData"
Write-Host "[phantomchat-deploy] OK — next PhantomChat launch will auto-enroll into org '{org_id}'."
"#,
        org_id = org_id,
        base_msi_filename = base_msi_filename,
        bootstrap_filename = bootstrap_filename
    )
}

fn render_readme(
    org_name: &str,
    org_id: &str,
    bootstrap_path: &Path,
    deploy_ps1_path: &Path,
    base_msi: &Path,
    lan_code: Option<&str>,
) -> String {
    let lan_line = match lan_code {
        Some(c) => format!("LAN auto-join code        : {}\n", c),
        None => String::from("LAN auto-join code        : (none — pure remote workforce)\n"),
    };
    format!(
        "PhantomChat enterprise deploy bundle
====================================

Org name    : {org_name}
Org ID      : {org_id}
{lan_line}
Files in this bundle:
  * {bootstrap}    — per-org pre-seed config
  * {deploy_ps1}    — Windows install + bootstrap-stage script
  * (you must also ship the upstream MSI: {base_msi})

Quick-start (single host):
  1. Copy all three files to the target PC.
  2. Right-click {deploy_ps1_name} -> Run with PowerShell (admin).
  3. Launch PhantomChat — the wizard is skipped, the org directory is
     pre-populated, and (if a LAN code is set) the install auto-joins the
     office mDNS group.

For SCCM / Intune / Group Policy recipes see
  tools/phantom-build-org-msi/README.md
",
        org_name = org_name,
        org_id = org_id,
        lan_line = lan_line,
        bootstrap = bootstrap_path.display(),
        deploy_ps1 = deploy_ps1_path.display(),
        deploy_ps1_name = deploy_ps1_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        base_msi = base_msi.display(),
    )
}
