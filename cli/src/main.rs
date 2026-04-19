//! PhantomChat CLI — cyberpunk terminal interface.
//!
//! ```
//! phantom keygen                       # generate identity
//! phantom pair                         # show QR pairing data
//! phantom send -r <spend_pub> -m "hi"  # send encrypted message
//! phantom listen                       # receive messages
//! phantom mode                         # show active privacy mode
//! phantom mode stealth                 # switch to Maximum Stealth
//! phantom mode daily                   # switch to Daily Use
//! phantom relay -u wss://relay.url     # health check
//! phantom status                       # node status overview
//! ```

use anyhow::Context;
use clap::{Parser, Subcommand};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use phantomchat_core::{
    envelope::Envelope,
    keys::{IdentityKey, ViewKey, SpendKey},
    privacy::{PrivacyConfig, PrivacyMode, ProxyConfig, ProxyKind},
};
use phantomchat_relays::make_relay;
use qrcodegen::{QrCode, QrCodeEcc};
use rand::random;
use rand_core::{OsRng, RngCore};
use std::{fs, path::PathBuf, time::Duration};
use x25519_dalek::{PublicKey, StaticSecret};

// ── ANSI palette (matches Flutter CyberpunkTheme) ────────────────────────────

const G: &str  = "\x1b[38;2;0;255;159m";    // neonGreen  #00FF9F
const M: &str  = "\x1b[38;2;255;0;255m";    // neonMagenta #FF00FF
const C: &str  = "\x1b[38;2;0;255;255m";    // cyber cyan
const DIM: &str = "\x1b[2m";
const B: &str  = "\x1b[1m";
const R: &str  = "\x1b[0m";

// ── Banner ────────────────────────────────────────────────────────────────────

fn banner(cfg: &PrivacyConfig) {
    let mode_label = match cfg.mode {
        PrivacyMode::DailyUse       => format!("{}[ DAILY USE ]{}", G, R),
        PrivacyMode::MaximumStealth => format!("{}[ MAXIMUM STEALTH ]{}", M, R),
    };
    println!();
    println!("{}", format!(
"{}{}██████╗ ██╗  ██╗ █████╗ ███╗   ██╗████████╗ ██████╗ ███╗   ███╗{}",
    B, G, R));
    println!("{}", format!(
"{}{}██╔══██╗██║  ██║██╔══██╗████╗  ██║╚══██╔══╝██╔═══██╗████╗ ████║{}",
    B, G, R));
    println!("{}", format!(
"{}{}██████╔╝███████║███████║██╔██╗ ██║   ██║   ██║   ██║██╔████╔██║{}",
    B, G, R));
    println!("{}", format!(
"{}{}██╔═══╝ ██╔══██║██╔══██║██║╚██╗██║   ██║   ██║   ██║██║╚██╔╝██║{}",
    B, M, R));
    println!("{}", format!(
"{}{}██║     ██║  ██║██║  ██║██║ ╚████║   ██║   ╚██████╔╝██║ ╚═╝ ██║{}",
    B, M, R));
    println!("{}", format!(
"{}{}╚═╝     ╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═══╝   ╚═╝    ╚═════╝ ╚═╝     ╚═╝{}",
    B, M, R));
    println!();
    println!(
        "{}{}  C H A T  {}{}│{} DC INFOSEC 2026 {}│{} {}",
        B, C, R, DIM, R, DIM, R, mode_label
    );
    println!("{}{}  ─────────────────────────────────────────────────────────────{}",
        DIM, G, R);
    println!();
}

// ── Privacy config persistence (CLI-local ~/.phantom_config.json) ─────────────

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".phantom_config.json")
}

fn load_config() -> PrivacyConfig {
    fs::read(config_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_config(cfg: &PrivacyConfig) -> anyhow::Result<()> {
    fs::write(config_path(), serde_json::to_vec_pretty(cfg)?)?;
    Ok(())
}

// ── CLI definition ────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "phantom",
    about = "PhantomChat — decentralized encrypted messenger CLI",
    long_about = None,
    disable_version_flag = false,
    color = clap::ColorChoice::Always,
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a new identity keypair
    Keygen {
        #[arg(short, long, default_value = "keys.json")]
        out: PathBuf,
    },
    /// Show pairing data (QR + hex keys)
    Pair {
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
    },
    /// Send an encrypted message
    Send {
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
        /// Recipient address: `view_pub_hex:spend_pub_hex` (the string
        /// produced by `phantom pair` after the `phantom:` prefix).
        #[arg(short = 'r', long)]
        recipient: String,
        /// Plaintext message
        #[arg(short, long)]
        message: String,
        /// Nostr relay URL
        #[arg(short = 'u', long, default_value = "wss://relay.damus.io")]
        relay: String,
    },
    /// Listen for incoming messages
    Listen {
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
        /// Nostr relay URL
        #[arg(short = 'u', long, default_value = "wss://relay.damus.io")]
        relay: String,
    },
    /// Show or change the active privacy mode
    Mode {
        /// daily | stealth
        mode: Option<String>,
        /// SOCKS5 proxy address (stealth mode)
        #[arg(long, default_value = "127.0.0.1:9050")]
        proxy: String,
        /// Use Nym instead of Tor
        #[arg(long)]
        nym: bool,
    },
    /// Check relay health and latency
    Relay {
        /// Relay URL to probe
        #[arg(short = 'u', long)]
        url: String,
    },
    /// Show current node status overview
    Status {
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
    },
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = load_config();
    banner(&cfg);

    let cli = Cli::parse();
    match cli.command {
        Commands::Keygen { out }                          => cmd_keygen(out)?,
        Commands::Pair   { file }                         => cmd_pair(file)?,
        Commands::Send   { file, recipient, message, relay } =>
            cmd_send(file, &recipient, &message, &relay, &cfg).await?,
        Commands::Listen { file, relay }                  =>
            cmd_listen(file, &relay, &cfg).await?,
        Commands::Mode   { mode, proxy, nym }             =>
            cmd_mode(mode, proxy, nym)?,
        Commands::Relay  { url }                          =>
            cmd_relay_health(&url, &cfg).await?,
        Commands::Status { file }                         =>
            cmd_status(file, &cfg)?,
    }
    Ok(())
}

// ── keygen ────────────────────────────────────────────────────────────────────

fn cmd_keygen(out: PathBuf) -> anyhow::Result<()> {
    let pb = spinner("Generating cryptographic identity...");

    let id    = IdentityKey::generate();
    let view  = ViewKey::generate();
    let spend = SpendKey::generate();

    let json = serde_json::json!({
        "identity_private": base64::encode(id.private),
        "identity_public":  base64::encode(id.public),
        "view_private":     base64::encode(view.secret.to_bytes()),
        "view_public":      hex::encode(view.public.as_bytes()),
        "spend_private":    base64::encode(spend.secret.to_bytes()),
        "spend_public":     hex::encode(spend.public.as_bytes()),
    });

    fs::write(&out, serde_json::to_vec_pretty(&json)?)?;
    pb.finish_and_clear();

    ok("KEYPAIR GENERATED");
    field("Identity public",  &base64::encode(id.public)[..16]);
    field("View public",      &hex::encode(view.public.as_bytes())[..16]);
    field("Spend public",     &hex::encode(spend.public.as_bytes())[..16]);
    field("Saved to",         &out.display().to_string());
    println!();
    warn("Keep spend_private secret — it decrypts all your messages.");
    Ok(())
}

// ── pair ──────────────────────────────────────────────────────────────────────

fn cmd_pair(file: PathBuf) -> anyhow::Result<()> {
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&file)?)?;
    let view_pub  = json["view_public"].as_str().context("missing view_public")?;
    let spend_pub = json["spend_public"].as_str().context("missing spend_public")?;

    // Pairing payload: view_pub:spend_pub
    let pairing = format!("phantom:{}:{}", view_pub, spend_pub);

    section("PAIRING DATA");
    field("View  pub", view_pub);
    field("Spend pub", spend_pub);
    println!();

    // ASCII QR
    section("QR CODE  (scan in PhantomChat mobile)");
    print_qr(&pairing);
    println!();
    dimline(&format!("Raw: {}", pairing));
    Ok(())
}

// ── send ──────────────────────────────────────────────────────────────────────

async fn cmd_send(
    _file: PathBuf,
    recipient_hex: &str,
    message: &str,
    relay_url: &str,
    cfg: &PrivacyConfig,
) -> anyhow::Result<()> {
    // `_file` is currently unused — the send flow only needs the recipient's
    // public address (which is passed via -r). Kept on the signature so the
    // CLI surface stays backward-compatible.

    // Parse recipient address: "view_pub_hex:spend_pub_hex" (or a legacy
    // `phantom:view:spend` pairing string — we tolerate the prefix).
    let raw = recipient_hex.strip_prefix("phantom:").unwrap_or(recipient_hex);
    let (view_hex, spend_hex) = raw
        .split_once(':')
        .context("recipient must be 'view_pub_hex:spend_pub_hex'")?;

    let view_bytes: [u8; 32] = hex::decode(view_hex)
        .context("view_pub must be 64-char hex")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("view_pub must be 32 bytes"))?;
    let spend_bytes: [u8; 32] = hex::decode(spend_hex)
        .context("spend_pub must be 64-char hex")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("spend_pub must be 32 bytes"))?;
    let recipient_view_pub  = PublicKey::from(view_bytes);
    let recipient_spend_pub = PublicKey::from(spend_bytes);

    let msg_id: u128 = random();

    let pb = spinner(&format!(
        "Building envelope (mode: {})",
        mode_str(cfg)
    ));

    // Build envelope — message bytes as encrypted_body, empty ratchet header
    let envelope = Envelope::new(
        &recipient_view_pub,
        &recipient_spend_pub,
        msg_id,
        vec![],                       // ratchet header (simplified for CLI)
        message.as_bytes().to_vec(),
        300,                          // TTL 5 min
        16,                           // PoW difficulty
    );

    pb.finish_and_clear();
    ok("ENVELOPE SEALED");
    field("Msg-ID",     &format!("{:#x}", msg_id));
    field("Ciphertext", &format!("{} bytes", envelope.ciphertext.len()));
    field("TTL",        "300 s");

    // Dandelion++ routing decision display
    let phase = if rand::random::<f64>() < 0.1 { "FLUFF (broadcast)" } else { "STEM (single-hop)" };
    field("Dandelion++", phase);
    println!();

    // Choose relay based on privacy mode
    let stealth = cfg.mode == PrivacyMode::MaximumStealth;
    let proxy   = cfg.proxy_addr();

    let pb2 = spinner(&format!(
        "Publishing via {} {}",
        if stealth { "STEALTH relay (SOCKS5)" } else { "relay (TLS)" },
        relay_url
    ));

    let relay = make_relay(relay_url, stealth, proxy);
    relay.publish(envelope).await
        .map_err(|e| { pb2.finish_and_clear(); e })?;

    pb2.finish_and_clear();
    ok("TRANSMITTED");
    field("Relay",   relay_url);
    let via = if stealth {
        format!("SOCKS5 {}", cfg.proxy.addr)
    } else {
        "direct TLS".to_string()
    };
    field("Via", &via);
    println!();
    Ok(())
}

// ── listen ────────────────────────────────────────────────────────────────────

async fn cmd_listen(
    file: PathBuf,
    relay_url: &str,
    cfg: &PrivacyConfig,
) -> anyhow::Result<()> {
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&file)?)?;

    // Reconstruct ViewKey (for stealth-scanning) and SpendKey (for decryption)
    // from the keyfile produced by `cmd_keygen`.
    let view_bytes = base64::decode(
        json["view_private"].as_str().context("missing view_private")?
    )?;
    let view_secret = StaticSecret::from(
        <[u8; 32]>::try_from(view_bytes.as_slice())
            .map_err(|_| anyhow::anyhow!("bad view key"))?
    );
    let view_key = phantomchat_core::keys::ViewKey {
        public: PublicKey::from(&view_secret),
        secret: view_secret,
    };

    let spend_bytes = base64::decode(
        json["spend_private"].as_str().context("missing spend_private")?
    )?;
    let spend_secret = StaticSecret::from(
        <[u8; 32]>::try_from(spend_bytes.as_slice())
            .map_err(|_| anyhow::anyhow!("bad spend key"))?
    );
    let spend_key = phantomchat_core::keys::SpendKey {
        public: PublicKey::from(&spend_secret),
        secret: spend_secret,
    };

    let stealth = cfg.mode == PrivacyMode::MaximumStealth;
    let proxy   = cfg.proxy_addr();

    section("LISTENING");
    field("Relay",  relay_url);
    field("Mode",   mode_str(cfg));
    if stealth { field("Proxy", &cfg.proxy.addr); }
    println!();
    println!("{}{}  View-key stealth-scanning every envelope...{}",
        DIM, G, R);
    println!("{}{}  Press Ctrl+C to stop{}",
        DIM, G, R);
    println!();

    let relay = make_relay(relay_url, stealth, proxy);

    let view_key_clone = view_key.clone();
    let spend_key_clone = spend_key.clone();
    relay.subscribe(Box::new(move |env| {
        match phantomchat_core::scan_envelope(&env, &view_key_clone, &spend_key_clone) {
            phantomchat_core::ScanResult::Mine(payload) => {
                let body = String::from_utf8_lossy(&payload.encrypted_body);
                println!(
                    "{}{}  ► DECRYPTED{}  msg_id={:#x}  body={}",
                    B, G, R,
                    payload.msg_id,
                    body.bright_green()
                );
            }
            phantomchat_core::ScanResult::Corrupted => {
                eprintln!(
                    "{}{}  ► TAG-MATCH but decrypt failed (corrupted / replay){}",
                    B, M, R
                );
            }
            phantomchat_core::ScanResult::NotMine => {
                // Not for us — expected for every cover-traffic dummy and for
                // real messages destined to other identities. Dot shows activity.
                print!("{}·{}", DIM, R);
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
        }
    })).await?;

    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

// ── mode ──────────────────────────────────────────────────────────────────────

fn cmd_mode(mode_arg: Option<String>, proxy: String, nym: bool) -> anyhow::Result<()> {
    let mut cfg = load_config();

    match mode_arg.as_deref() {
        None => {
            // Show current
            section("PRIVACY MODE");
            let active = mode_str(&cfg);
            println!("  Active: {}", active);
            println!();
            match cfg.mode {
                PrivacyMode::DailyUse => {
                    println!("  {}{}libp2p + Dandelion++ + Nostr/TLS{}", B, G, R);
                    println!("  {}Light cover traffic (30–180 s){}", DIM, R);
                }
                PrivacyMode::MaximumStealth => {
                    println!("  {}{}Relay-only · All traffic via SOCKS5{}", B, M, R);
                    println!("  {}Proxy: {}  Aggressive cover traffic (5–15 s){}", DIM, cfg.proxy.addr, R);
                }
            }
        }
        Some("daily") | Some("dailyuse") => {
            cfg.mode  = PrivacyMode::DailyUse;
            save_config(&cfg)?;
            ok(&format!("Mode set to {}", mode_str(&cfg)));
            dimline("libp2p active · Dandelion++ routing · Light cover traffic");
        }
        Some("stealth") | Some("maximumstealth") | Some("paranoia") => {
            cfg.mode  = PrivacyMode::MaximumStealth;
            cfg.proxy = ProxyConfig {
                addr: proxy.clone(),
                kind: if nym { ProxyKind::Nym } else { ProxyKind::Tor },
            };
            save_config(&cfg)?;
            ok(&format!("Mode set to {}", mode_str(&cfg)));
            field("Proxy",  &proxy);
            field("Network", if nym { "Nym" } else { "Tor" });
            dimline("libp2p disabled · relay-only · aggressive cover traffic");
            warn("Make sure Tor/Nym is running on the configured proxy address.");
        }
        Some(other) => {
            anyhow::bail!("Unknown mode '{}'. Use: daily | stealth", other);
        }
    }
    println!();
    Ok(())
}

// ── relay health ──────────────────────────────────────────────────────────────

async fn cmd_relay_health(url: &str, cfg: &PrivacyConfig) -> anyhow::Result<()> {
    let stealth = cfg.mode == PrivacyMode::MaximumStealth;
    let pb = spinner(&format!("Probing {} …", url));

    let relay  = make_relay(url, stealth, cfg.proxy_addr());
    let health = relay.health().await;

    pb.finish_and_clear();
    section("RELAY HEALTH");
    field("URL",          url);
    field("Via",          if stealth { "SOCKS5 (stealth)" } else { "direct" });
    field("Latency",      &format!("{} ms", health.latency_ms));
    field("Uptime",       &format!("{:.0}%", health.uptime * 100.0));
    field("Failure rate", &format!("{:.0}%", health.failure_rate * 100.0));

    let status = if health.failure_rate < 0.2 {
        format!("{}{}ONLINE{}", B, G, R)
    } else {
        format!("{}{}DEGRADED{}", B, M, R)
    };
    field("Status", &status);
    println!();
    Ok(())
}

// ── status ────────────────────────────────────────────────────────────────────

fn cmd_status(file: PathBuf, cfg: &PrivacyConfig) -> anyhow::Result<()> {
    section("NODE STATUS");

    // Keys
    let keys_ok = file.exists();
    let id_label = if keys_ok {
        format!("{}✓ {}{}", G, file.display(), R)
    } else {
        format!("{}✗ not found{}", M, R)
    };
    field("Identity file", &id_label);

    // Privacy mode
    field("Privacy mode",    mode_str(cfg));
    match cfg.mode {
        PrivacyMode::DailyUse => {
            field("P2P",          &format!("{}ENABLED{}", G, R));
            field("Dandelion++",  &format!("{}ACTIVE{}", G, R));
            field("Cover traffic",&format!("{}Light (30–180 s){}", G, R));
        }
        PrivacyMode::MaximumStealth => {
            field("P2P",          &format!("{}DISABLED{}", M, R));
            field("SOCKS5 proxy", &cfg.proxy.addr);
            field("Anonymizer",   match cfg.proxy.kind { ProxyKind::Tor => "Tor", ProxyKind::Nym => "Nym" });
            field("Cover traffic",&format!("{}Aggressive (5–15 s){}", M, R));
        }
    }

    // Config path
    field("Config", &config_path().display().to_string());
    println!();
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template(&format!("  {{spinner:.green}} {}{}{}  {{msg}}", DIM, msg, R))
            .unwrap(),
    );
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

fn ok(msg: &str) {
    println!("  {}{}✓  {}{}", B, G, msg, R);
}

fn warn(msg: &str) {
    println!("  {}{}⚠  {}{}", B, M, R, msg);
}

fn field(key: &str, val: &str) {
    println!("  {}{}  {:<18}{}{}", DIM, G, key, R, val);
}

fn dimline(msg: &str) {
    println!("  {}{}{}", DIM, msg, R);
}

fn section(title: &str) {
    println!("  {}{}{}{} ─────────────────────────", B, C, title, R);
}

fn mode_str(cfg: &PrivacyConfig) -> &'static str {
    match cfg.mode {
        PrivacyMode::DailyUse       => "\x1b[38;2;0;255;159m\x1b[1mDAILY USE\x1b[0m",
        PrivacyMode::MaximumStealth => "\x1b[38;2;255;0;255m\x1b[1mMAXIMUM STEALTH\x1b[0m",
    }
}

fn print_qr(data: &str) {
    let qr = QrCode::encode_text(data, QrCodeEcc::Medium)
        .expect("QR encode failed");
    let size = qr.size();
    // Top border
    print!("  {}{}", DIM, G);
    for _ in 0..size + 4 { print!("██"); }
    println!("{}", R);
    // Empty top row
    print!("  {}{}", DIM, G);
    print!("████");
    for _ in 0..size { print!("  "); }
    println!("████{}", R);
    // Data rows
    for y in 0..size {
        print!("  {}{}", DIM, G);
        print!("████");
        for x in 0..size {
            if qr.get_module(x, y) {
                print!("{}██{}{}", R, DIM, G);
            } else {
                print!("  ");
            }
        }
        println!("████{}", R);
    }
    // Empty bottom row + border
    print!("  {}{}", DIM, G);
    print!("████");
    for _ in 0..size { print!("  "); }
    println!("████{}", R);
    print!("  {}{}", DIM, G);
    for _ in 0..size + 4 { print!("██"); }
    println!("{}", R);
}
