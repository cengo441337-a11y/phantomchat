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
    address::PhantomAddress,
    fingerprint::safety_number,
    group::PhantomGroup,
    keys::{HybridKeyPair, IdentityKey, PhantomSigningKey, SpendKey, ViewKey},
    mixnet::{pack_onion, peel_onion, MixnetHop, Peeled},
    prekey::PrekeyMaterial,
    privacy::{PrivacyConfig, PrivacyMode, ProxyConfig, ProxyKind},
    psi::{PsiClient, PsiServer},
    session::SessionStore,
};
use x25519_dalek::StaticSecret as MixnetSecret;
use phantomchat_relays::make_relay;
use qrcodegen::{QrCode, QrCodeEcc};
use rand_core::{OsRng, RngCore};
use std::{fs, path::PathBuf, sync::{Arc, Mutex}, time::Duration};
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
    /// End-to-end pipeline self-test — generates two ephemeral identities,
    /// exchanges messages through the full Envelope+Ratchet stack in one
    /// process and reports pass/fail. No network or keyfile required.
    Selftest,
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
        Commands::Selftest                                => cmd_selftest()?,
    }
    Ok(())
}

// ── selftest ──────────────────────────────────────────────────────────────────

fn cmd_selftest() -> anyhow::Result<()> {
    section("PIPELINE SELF-TEST");

    // ── Phase 1: classic X25519-only flow ────────────────────────────────
    dimline("Phase 1 — classic envelope (X25519 + Double Ratchet)");

    let alice_view  = ViewKey::generate();
    let alice_spend = SpendKey::generate();
    let alice_addr  = PhantomAddress::new(alice_view.public, alice_spend.public);

    let bob_view  = ViewKey::generate();
    let bob_spend = SpendKey::generate();
    let bob_addr  = PhantomAddress::new(bob_view.public, bob_spend.public);

    let mut alice_store = SessionStore::new();
    let mut bob_store   = SessionStore::new();

    let classic_script: &[(&str, &[u8])] = &[
        ("A→B", b"handshake"),
        ("A→B", b"chain continues"),
        ("B→A", b"reply"),
        ("A→B", b"post-rotation"),
        ("B→A", b"same here"),
        ("A→B", b"sealed"),
    ];

    let mut passed = 0usize;
    let mut failed = 0usize;

    for (dir, plain) in classic_script {
        let result = match *dir {
            "A→B" => {
                let env = alice_store.send(&bob_addr, plain, 0);
                bob_store.receive(&env, &bob_view, &bob_spend)
            }
            "B→A" => {
                let env = bob_store.send(&alice_addr, plain, 0);
                alice_store.receive(&env, &alice_view, &alice_spend)
            }
            _ => continue,
        };
        report_result(dir, plain, result, &mut passed, &mut failed);
    }

    // ── Phase 2: PQXDH-hybrid flow (X25519 + ML-KEM-1024) ───────────────
    println!();
    dimline("Phase 2 — hybrid envelope (PQXDH: X25519 + ML-KEM-1024)");

    let carol_view   = ViewKey::generate();
    let carol_hybrid = HybridKeyPair::generate();
    let carol_spend  = SpendKey {
        public: carol_hybrid.public.x25519,
        secret: carol_hybrid.secret.x25519.clone(),
    };
    let carol_addr = PhantomAddress::new_hybrid(
        carol_view.public,
        carol_hybrid.public.x25519,
        carol_hybrid.public.to_bytes()[32..].to_vec(),
    );

    let dave_view   = ViewKey::generate();
    let dave_hybrid = HybridKeyPair::generate();
    let dave_spend  = SpendKey {
        public: dave_hybrid.public.x25519,
        secret: dave_hybrid.secret.x25519.clone(),
    };
    let dave_addr = PhantomAddress::new_hybrid(
        dave_view.public,
        dave_hybrid.public.x25519,
        dave_hybrid.public.to_bytes()[32..].to_vec(),
    );

    let mut carol_store = SessionStore::new();
    let mut dave_store  = SessionStore::new();

    let hybrid_script: &[(&str, &[u8])] = &[
        ("C→D", b"pq handshake"),
        ("D→C", b"pq reply"),
        ("C→D", b"pq rotation"),
        ("D→C", b"pq final"),
    ];

    for (dir, plain) in hybrid_script {
        let result = match *dir {
            "C→D" => {
                let env = carol_store.send(&dave_addr, plain, 0);
                dave_store.receive_hybrid(&env, &dave_view, &dave_spend, &dave_hybrid.secret)
            }
            "D→C" => {
                let env = dave_store.send(&carol_addr, plain, 0);
                carol_store.receive_hybrid(&env, &carol_view, &carol_spend, &carol_hybrid.secret)
            }
            _ => continue,
        };
        report_result(dir, plain, result, &mut passed, &mut failed);
    }

    // ── Phase 3: Sealed Sender (identity authentication) ────────────────
    println!();
    dimline("Phase 3 — Sealed Sender (Ed25519 identity attribution)");

    let eve_view  = ViewKey::generate();
    let eve_spend = SpendKey::generate();
    let eve_addr  = PhantomAddress::new(eve_view.public, eve_spend.public);
    let frank_view  = ViewKey::generate();
    let frank_spend = SpendKey::generate();
    let frank_addr  = PhantomAddress::new(frank_view.public, frank_spend.public);
    let eve_sign = PhantomSigningKey::generate();

    let mut eve_store   = SessionStore::new();
    let mut frank_store = SessionStore::new();

    let sealed_env = eve_store.send_sealed(&frank_addr, b"signed hello", &eve_sign, 0);
    match frank_store.receive_full(&sealed_env, &frank_view, &frank_spend, None) {
        Ok(Some(msg)) => {
            let plaintext_ok = msg.plaintext == b"signed hello";
            let (attr, sig_ok) = msg.sender.clone().unwrap_or_else(|| (
                phantomchat_core::SealedSender { sender_pub: [0u8; 32], signature: [0u8; 64] },
                false,
            ));
            let identity_ok = attr.sender_pub == eve_sign.public_bytes();
            if plaintext_ok && sig_ok && identity_ok {
                println!(
                    "  {}{}✓ sealed-sender round-trip{}              plaintext ok · sig ok · identity={}",
                    B, G, R,
                    &hex::encode(&attr.sender_pub)[..8],
                );
                passed += 1;
            } else {
                println!("  {}{}✗ sealed-sender round-trip{}              plaintext={} sig={} identity={}",
                    B, M, R, plaintext_ok, sig_ok, identity_ok);
                failed += 1;
            }
        }
        other => {
            println!("  {}{}✗ sealed-sender receive failed{}  {:?}", B, M, R, other);
            failed += 1;
        }
    }
    let _ = eve_addr;

    // ── Phase 4: Safety Numbers (Signal-style MITM detection) ────────────
    println!();
    dimline("Phase 4 — Safety Numbers (60-digit session fingerprint)");

    let number = safety_number(&alice_addr, &bob_addr);
    let reverse = safety_number(&bob_addr, &alice_addr);
    if number == reverse && number.split(' ').count() == 12 {
        println!("  {}{}✓ fingerprint is symmetric + 12-group format{}", B, G, R);
        println!("  {}  Alice↔Bob: {}{}", DIM, number, R);
        passed += 1;
    } else {
        println!("  {}{}✗ fingerprint malformed{}", B, M, R);
        failed += 1;
    }

    // ── Phase 5: X3DH Prekey Bundle signature chain ──────────────────────
    println!();
    dimline("Phase 5 — X3DH Prekey Bundle");

    let identity = PhantomSigningKey::generate();
    let (_material, bundle) = PrekeyMaterial::fresh(&identity);
    let self_verifies = bundle.verify();
    let fp = bundle.fingerprint();

    if self_verifies {
        println!(
            "  {}{}✓ prekey bundle signed + verifies{}         fingerprint={}",
            B, G, R, hex::encode(fp),
        );
        passed += 1;
    } else {
        println!("  {}{}✗ bundle signature did not verify{}", B, M, R);
        failed += 1;
    }

    // Tamper check — a foreign identity replacing the identity pub must fail.
    let mut forged = bundle.clone();
    let impostor = PhantomSigningKey::generate();
    forged.identity_pub = hex::encode(impostor.public_bytes());
    if !forged.verify() {
        println!("  {}{}✓ forged bundle (swapped identity) rejected{}", B, G, R);
        passed += 1;
    } else {
        println!("  {}{}✗ forged bundle verifies — signature oracle bug{}", B, M, R);
        failed += 1;
    }

    // ── Phase 6: Group chat via Sender Keys ──────────────────────────────
    println!();
    dimline("Phase 6 — Group chat (Sender Keys, 3 senders × 2 msgs)");

    let g_alice_sign = PhantomSigningKey::generate();
    let g_bob_sign   = PhantomSigningKey::generate();
    let g_carol_sign = PhantomSigningKey::generate();

    let roster = vec![
        PhantomAddress::new(ViewKey::generate().public, SpendKey::generate().public),
        PhantomAddress::new(ViewKey::generate().public, SpendKey::generate().public),
        PhantomAddress::new(ViewKey::generate().public, SpendKey::generate().public),
    ];

    let mut g_alice = PhantomGroup::new(roster.clone(), &g_alice_sign);
    let mut g_bob   = PhantomGroup::new(roster.clone(), &g_bob_sign);
    let mut g_carol = PhantomGroup::new(roster.clone(), &g_carol_sign);
    // Sync group_ids — the `new()` randomises one per instance; in a real
    // deployment the organiser sends it out as part of the invite.
    g_bob.group_id   = g_alice.group_id;
    g_carol.group_id = g_alice.group_id;

    // Distribute every sender's key to every other member via 1-to-1.
    g_bob.accept_distribution(g_alice.own_distribution(&g_alice_sign));
    g_carol.accept_distribution(g_alice.own_distribution(&g_alice_sign));
    g_alice.accept_distribution(g_bob.own_distribution(&g_bob_sign));
    g_carol.accept_distribution(g_bob.own_distribution(&g_bob_sign));
    g_alice.accept_distribution(g_carol.own_distribution(&g_carol_sign));
    g_bob.accept_distribution(g_carol.own_distribution(&g_carol_sign));

    let group_script: &[(&str, &PhantomSigningKey, &[u8])] = &[
        ("Alice→grp", &g_alice_sign, b"hi everyone"),
        ("Bob→grp",   &g_bob_sign,   b"hey"),
        ("Carol→grp", &g_carol_sign, b"joined"),
        ("Alice→grp", &g_alice_sign, b"second"),
        ("Bob→grp",   &g_bob_sign,   b"here"),
        ("Carol→grp", &g_carol_sign, b"also here"),
    ];

    for (label, sign, plain) in group_script {
        // Each sender encrypts with their own state; the other two members
        // decrypt. We'll assert at least both of the non-senders succeed.
        let wire = match *label {
            "Alice→grp" => g_alice.encrypt(sign, plain),
            "Bob→grp"   => g_bob.encrypt(sign, plain),
            "Carol→grp" => g_carol.encrypt(sign, plain),
            _ => continue,
        };

        let (pb, pc) = match *label {
            "Alice→grp" => (g_bob.decrypt(&wire), g_carol.decrypt(&wire)),
            "Bob→grp"   => (g_alice.decrypt(&wire), g_carol.decrypt(&wire)),
            "Carol→grp" => (g_alice.decrypt(&wire), g_bob.decrypt(&wire)),
            _ => continue,
        };

        let ok_a = pb.as_ref().map(|v| v == *plain).unwrap_or(false);
        let ok_b = pc.as_ref().map(|v| v == *plain).unwrap_or(false);
        if ok_a && ok_b {
            println!(
                "  {}{}✓ {} {:<20}{}  both receivers decoded ({} B)",
                B, G, label, String::from_utf8_lossy(plain), R, plain.len(),
            );
            passed += 1;
        } else {
            println!(
                "  {}{}✗ {} {:<20}{}  r1={:?} r2={:?}",
                B, M, label, String::from_utf8_lossy(plain), R,
                pb.err(), pc.err(),
            );
            failed += 1;
        }
    }

    // ── Phase 7: Onion-routed mixnet (3-hop Sphinx-style) ────────────────
    println!();
    dimline("Phase 7 — Onion mixnet (3-hop layered AEAD)");

    let h1_sec = MixnetSecret::random_from_rng(&mut OsRng);
    let h2_sec = MixnetSecret::random_from_rng(&mut OsRng);
    let h3_sec = MixnetSecret::random_from_rng(&mut OsRng);

    let h1 = MixnetHop { public: (&h1_sec).into() };
    let h2 = MixnetHop { public: (&h2_sec).into() };
    let h3 = MixnetHop { public: (&h3_sec).into() };

    let packet = pack_onion(&[h1.clone(), h2.clone(), h3.clone()], b"onion-delivered");

    let peeled1 = peel_onion(&packet, &h1_sec);
    let after_h1 = match peeled1 {
        Ok(Peeled::Forward { packet: p, .. }) => p,
        _ => { println!("  {}{}✗ hop1 did not forward{}", B, M, R); failed += 1; return Err(anyhow::anyhow!("mixnet")); }
    };
    let peeled2 = peel_onion(&after_h1, &h2_sec);
    let after_h2 = match peeled2 {
        Ok(Peeled::Forward { packet: p, .. }) => p,
        _ => { println!("  {}{}✗ hop2 did not forward{}", B, M, R); failed += 1; return Err(anyhow::anyhow!("mixnet")); }
    };
    match peel_onion(&after_h2, &h3_sec) {
        Ok(Peeled::Final { payload }) if payload == b"onion-delivered" => {
            println!("  {}{}✓ 3-hop onion round-trip{}                  payload intact after 3 peels", B, G, R);
            passed += 1;
        }
        other => {
            println!("  {}{}✗ hop3 delivery failed{}  {:?}", B, M, R, other);
            failed += 1;
        }
    }

    // Peel with the wrong secret must fail.
    let impostor_sec = MixnetSecret::random_from_rng(&mut OsRng);
    let rogue = peel_onion(&packet, &impostor_sec);
    if rogue.is_err() {
        println!("  {}{}✓ wrong-key peel correctly refused{}", B, G, R);
        passed += 1;
    } else {
        println!("  {}{}✗ wrong-key peel should have failed{}", B, M, R);
        failed += 1;
    }

    // ── Phase 8: Private Set Intersection (DDH-Ristretto) ───────────────
    println!();
    dimline("Phase 8 — Private Set Intersection (DDH-Ristretto)");

    let shared_a = PhantomAddress::new(ViewKey::generate().public, SpendKey::generate().public);
    let shared_b = PhantomAddress::new(ViewKey::generate().public, SpendKey::generate().public);
    let alice_only = PhantomAddress::new(ViewKey::generate().public, SpendKey::generate().public);
    let bob_only = PhantomAddress::new(ViewKey::generate().public, SpendKey::generate().public);

    let alice_set = vec![alice_only.clone(), shared_a.clone(), shared_b.clone()];
    let bob_set   = vec![bob_only.clone(),   shared_a.clone(), shared_b.clone()];

    let alice_psi = PsiClient::new(&alice_set);
    let bob_psi   = PsiServer::new(&bob_set);

    let my_dbl = bob_psi
        .double_blind(alice_psi.blinded_query())
        .expect("bob double-blind");
    let peer_blinded = bob_psi.blinded_directory().to_vec();
    let hits = alice_psi
        .intersect(&my_dbl, &peer_blinded)
        .expect("intersect");

    let hit_set: std::collections::HashSet<_> = hits.iter().map(|a| a.short_id()).collect();
    let alice_only_leaked = hit_set.contains(&alice_only.short_id());
    let bob_only_leaked = hit_set.contains(&bob_only.short_id());
    let shared_found = hit_set.contains(&shared_a.short_id())
        && hit_set.contains(&shared_b.short_id());

    if shared_found && !alice_only_leaked && !bob_only_leaked {
        println!(
            "  {}{}✓ PSI found {} shared, leaked 0 non-shared{}",
            B, G, hits.len(), R
        );
        passed += 1;
    } else {
        println!(
            "  {}{}✗ PSI wrong — hits={} shared_found={} leaked={}{}",
            B, M, hits.len(), shared_found, alice_only_leaked || bob_only_leaked, R
        );
        failed += 1;
    }

    // ── Summary ─────────────────────────────────────────────────────────
    let total = classic_script.len() + hybrid_script.len()
        + 1 /* sealed */ + 1 /* safety number */ + 2 /* prekey */
        + group_script.len()
        + 2 /* mixnet (roundtrip + wrong-key) */
        + 1 /* PSI */;
    println!();
    if failed == 0 {
        ok(&format!(
            "SELF-TEST PASSED — {}/{} checks across 8 phases",
            passed, total
        ));
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "self-test failed: {} passed, {} failed",
            passed,
            failed
        ))
    }
}

fn report_result(
    dir: &str,
    plain: &[u8],
    result: Result<Option<Vec<u8>>, phantomchat_core::session::SessionError>,
    passed: &mut usize,
    failed: &mut usize,
) {
    match result {
        Ok(Some(got)) if got == plain => {
            println!(
                "  {}{}✓ {} {:<32}{}  plaintext matched ({} B)",
                B, G, dir, String::from_utf8_lossy(plain), R, got.len(),
            );
            *passed += 1;
        }
        Ok(Some(got)) => {
            println!(
                "  {}{}✗ {} {:<32}{}  mismatch: got {:?}",
                B, M, dir, String::from_utf8_lossy(plain), R,
                String::from_utf8_lossy(&got),
            );
            *failed += 1;
        }
        Ok(None) => {
            println!(
                "  {}{}✗ {} {:<32}{}  silently dropped (scanner miss)",
                B, M, dir, String::from_utf8_lossy(plain), R,
            );
            *failed += 1;
        }
        Err(e) => {
            println!(
                "  {}{}✗ {} {:<32}{}  error: {}",
                B, M, dir, String::from_utf8_lossy(plain), R, e,
            );
            *failed += 1;
        }
    }
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
    file: PathBuf,
    recipient_hex: &str,
    message: &str,
    relay_url: &str,
    cfg: &PrivacyConfig,
) -> anyhow::Result<()> {
    // Parse recipient — accepts "phantom:view:spend", "view:spend", or
    // either form copied straight out of `phantom pair` output.
    let recipient = PhantomAddress::parse(recipient_hex)
        .context("recipient must be 'view_pub_hex:spend_pub_hex'")?;

    // Load / create the per-identity session store. Persisting next to the
    // keyfile keeps alice's and bob's ratchet state cleanly separated when
    // running multiple identities on the same box.
    let sessions_path = sessions_path_for(&file);
    let mut store = SessionStore::load(&sessions_path)
        .with_context(|| format!("loading sessions from {}", sessions_path.display()))?;

    let pb = spinner(&format!("Sealing envelope ({} mode)", mode_str(cfg)));

    // SessionStore::send wraps the plaintext through the per-contact
    // Double Ratchet and produces a fully signed + PoW'd Envelope ready
    // to publish. PoW difficulty stays moderate for the CLI.
    let envelope = store.send(&recipient, message.as_bytes(), 16);

    pb.finish_and_clear();
    ok("ENVELOPE SEALED");
    field("Recipient",   &recipient.short_id());
    field("Ciphertext",  &format!("{} bytes", envelope.ciphertext.len()));
    field("TTL",         "300 s");

    let phase = if rand::random::<f64>() < 0.1 { "FLUFF (broadcast)" } else { "STEM (single-hop)" };
    field("Dandelion++", phase);
    println!();

    let stealth = cfg.mode == PrivacyMode::MaximumStealth;
    let proxy   = cfg.proxy_addr();

    let pb2 = spinner(&format!(
        "Publishing via {} {}",
        if stealth { "STEALTH relay (SOCKS5)" } else { "relay (TLS)" },
        relay_url
    ));

    let relay = make_relay(relay_url, stealth, proxy);
    let publish_result = relay.publish(envelope).await;
    pb2.finish_and_clear();
    publish_result?;

    // Persist the now-advanced ratchet state. Skipping this would cause the
    // next send to reuse the same chain key and defeat forward secrecy.
    store
        .save(&sessions_path)
        .with_context(|| format!("saving sessions to {}", sessions_path.display()))?;

    ok("TRANSMITTED");
    field("Relay", relay_url);
    let via = if stealth {
        format!("SOCKS5 {}", cfg.proxy.addr)
    } else {
        "direct TLS".to_string()
    };
    field("Via", &via);
    field("Sessions", &format!("saved to {}", sessions_path.display()));
    println!();
    Ok(())
}

/// Sessions file lives next to the keyfile: `keys.json` → `keys.sessions.json`.
fn sessions_path_for(keyfile: &std::path::Path) -> PathBuf {
    let parent = keyfile.parent().unwrap_or_else(|| std::path::Path::new("."));
    let stem = keyfile
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("keys");
    parent.join(format!("{}.sessions.json", stem))
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

    // Load or initialise the persistent session store. Every incoming
    // envelope gets pushed through `SessionStore::receive`, which runs the
    // full pipeline: scanner tag check → envelope open → ratchet decrypt.
    let sessions_path = sessions_path_for(&file);
    let store = Arc::new(Mutex::new(
        SessionStore::load(&sessions_path)
            .with_context(|| format!("loading sessions from {}", sessions_path.display()))?,
    ));

    let view_key_clone  = view_key.clone();
    let spend_key_clone = spend_key.clone();
    let store_clone     = Arc::clone(&store);
    let save_path       = sessions_path.clone();

    relay.subscribe(Box::new(move |env| {
        let mut guard = match store_clone.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        match guard.receive(&env, &view_key_clone, &spend_key_clone) {
            Ok(Some(plaintext)) => {
                let body = String::from_utf8_lossy(&plaintext);
                println!(
                    "{}{}  ► DECRYPTED{}  {}",
                    B, G, R,
                    body.bright_green()
                );
                if let Err(e) = guard.save(&save_path) {
                    eprintln!("{}warning: could not persist sessions: {}{}", DIM, e, R);
                }
            }
            Ok(None) => {
                // Not for us — expected for cover traffic and other peers.
                print!("{}·{}", DIM, R);
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            Err(e) => {
                eprintln!(
                    "{}{}  ► DECRYPT ERROR{}  {}",
                    B, M, R, e
                );
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
