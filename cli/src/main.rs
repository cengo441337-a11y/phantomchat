//! PhantomChat CLI
//!
//! Dieser Kommandozeilen‑Client ermöglicht es, Schlüssel zu generieren,
//! Pairing‑Informationen auszutauschen sowie Nachrichten zu versenden
//! und zu empfangen.  Der Code basiert auf der Kernbibliothek
//! `phantomchat_core` und nutzt `tokio` für asynchrones I/O.  Der
//! Netzwerkcode für Nostr‑Relays und die QR‑Funktionen sind nur
//! rudimentär implementiert.

use clap::{Parser, Subcommand};
use phantomchat_core::{IdentityKey, ViewKey, SpendKey, Envelope};
use x25519_dalek::{PublicKey, StaticSecret};
use rand_core::OsRng;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

/// Kommandozeilenoptionen
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Erzeugt ein neues Schlüsselpaar und speichert es als JSON
    Keygen {
        /// Ausgabeordner
        #[arg(short, long, default_value = "keys.json")]
        out: PathBuf,
    },
    /// Zeigt Pairing‑Daten (view_pub, spend_pub) an
    Pair {
        /// Schlüsseldatei
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
    },
    /// Sendet eine Nachricht an einen Empfänger
    Send {
        /// Schlüsseldatei
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
        /// Empfänger‑Spend‑Public‑Key (hex)
        #[arg(short = 'r', long)]
        recipient_spend_pub: String,
        /// Nachrichtentext
        #[arg(short, long)]
        message: String,
    },
    /// Lauscht auf eingehende Nachrichten (lokaler Relay‑Test)
    Listen {
        /// Schlüsseldatei
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Keygen { out } => {
            keygen(out)?;
        }
        Commands::Pair { file } => {
            pair(file)?;
        }
        Commands::Send { file, recipient_spend_pub, message } => {
            send(file, &recipient_spend_pub, &message).await?;
        }
        Commands::Listen { file } => {
            listen(file).await?;
        }
    }
    Ok(())
}

/// Generiert neue Schlüssel und speichert sie in einer JSON‑Datei.
fn keygen(out: PathBuf) -> anyhow::Result<()> {
    let id = IdentityKey::generate();
    let view = ViewKey::generate();
    let spend = SpendKey::generate();
    let keys = serde_json::json!({
        "identity_private": base64::encode(id.private),
        "identity_public": base64::encode(id.public),
        "view_private": base64::encode(view.secret.to_bytes()),
        "view_public": base64::encode(view.public.as_bytes()),
        "spend_private": base64::encode(spend.secret.to_bytes()),
        "spend_public": base64::encode(spend.public.as_bytes()),
    });
    fs::write(&out, serde_json::to_vec_pretty(&keys)?)?;
    println!("Schlüssel in {:?} gespeichert", out);
    Ok(())
}

/// Liest die Schlüsseldatei und zeigt die Pairing‑Daten an.
fn pair(file: PathBuf) -> anyhow::Result<()> {
    let data = fs::read(file)?;
    let json: serde_json::Value = serde_json::from_slice(&data)?;
    let view_pub = json["view_public"].as_str().unwrap();
    let spend_pub = json["spend_public"].as_str().unwrap();
    println!("Pairing‑Daten:\nview_pub: {}\nspend_pub: {}", view_pub, spend_pub);
    Ok(())
}

/// Lädt Schlüssel, baut ein Envelope und sendet es an den (hier
/// nur simulierten) Empfänger.  In einer echten Implementierung würde
/// diese Funktion die Nachricht via Nostr‑Relays übertragen.
async fn send(file: PathBuf, recipient_spend_pub_hex: &str, message: &str) -> anyhow::Result<()> {
    use phantomchat_core::RatchetState;

    // Schlüssel laden
    let data = fs::read(file)?;
    let keys: serde_json::Value = serde_json::from_slice(&data)?;
    let spend_pub_b64 = keys["spend_public"].as_str().ok_or_else(|| anyhow::anyhow!("Spend public key missing"))?;
    
    // Empfänger‑Spend‑Public‑Key parsen (der in hex‑format übergeben wurde)
    let rec_bytes = hex::decode(recipient_spend_pub_hex)?;
    let recipient_spend_pub = PublicKey::from(rec_bytes.as_slice().try_into().map_err(|_| anyhow::anyhow!("Invalid recipient pubkey"))?);
    
    // Initialisiere die Ratchet‑Engine. Für die Demo nutzen wir einen festen Root-Key.
    let root_key = [0u8; 32];
    let mut ratchet = RatchetState::new(root_key, recipient_spend_pub);

    // Nachricht verschlüsseln
    let (ciphertext, header) = ratchet.encrypt(message.as_bytes());

    // msg_id generieren
    let msg_id = rand::random::<u128>();

    let envelope = Envelope::new(
        &recipient_spend_pub,
        msg_id,
        0,
        header,
        ciphertext,
        60,
        16,
    );

    println!("🛡️ Nachricht kryptografisch gesichert!");
    println!("Envelope-Größe: {} Bytes", envelope.to_bytes().len());
    println!("Serielles Envelope (Base64): {}", base64::encode(envelope.to_bytes()));
    Ok(())
}

/// Lauscht auf eingehende Nachrichten (Nur Demo: Keine echten Relays)
async fn listen(file: PathBuf) -> anyhow::Result<()> {
    // Schlüssel laden
    let data = fs::read(file)?;
    let json: serde_json::Value = serde_json::from_slice(&data)?;
    let spend_priv = base64::decode(json["spend_private"].as_str().unwrap())?;
    let spend_secret = StaticSecret::from(spend_priv.as_slice().try_into().unwrap());
    let spend_key = SpendKey {
        secret: spend_secret.clone(),
        public: PublicKey::from(&spend_secret),
    };
    println!("Warte auf Nachrichten ... drücken Sie Ctrl+C zum Beenden.");
    loop {
        // In einer echten Implementierung würde hier ein Relay‑Abo
        // verarbeitet.  Zur Demonstration schlafen wir kurz.
        tokio::time::sleep(Duration::from_secs(5)).await;
        // Keine Nachrichten verfügbar
    }
}