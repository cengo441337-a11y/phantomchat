//! `phantom group` subcommand — Sender-Keys group chat operations.
//!
//! Sender-Keys are the persistable primitive for small groups (≤10 members).
//! For larger groups / stronger forward-secrecy guarantees, use MLS via the
//! library API (currently in-process only — CLI binding intentionally omitted
//! until openmls storage is wired).

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use clap::Subcommand;
use colored::Colorize;
use phantomchat_core::{
    address::PhantomAddress,
    group::{GroupError, PhantomGroup, SenderKeyDistribution},
    keys::PhantomSigningKey,
};
use std::{fs, path::PathBuf};

#[derive(serde::Serialize, serde::Deserialize)]
struct GroupInvite {
    group_id: [u8; 16],
    members: Vec<PhantomAddress>,
}

#[derive(Subcommand)]
pub enum GroupAction {
    /// Create a new Sender-Keys group. Emits an invite blob for other members.
    Create {
        /// Identity file produced by `phantom keygen`
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
        /// Comma-separated member addresses in `view_hex:spend_hex` (or `phantom:…`) format.
        /// Include your own address too.
        #[arg(short, long)]
        members: String,
        /// Output path for the serialized group state
        #[arg(short, long, default_value = "group.json")]
        out: PathBuf,
        /// Path to write the shareable invite blob (base64). Other members
        /// pass this to `phantom group join` to get matching local state.
        #[arg(short = 'i', long, default_value = "group_invite.b64")]
        invite_out: PathBuf,
    },
    /// Join an existing group using the invite blob the creator produced.
    Join {
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
        /// Path to the invite blob (base64 file from `group create`)
        #[arg(short, long)]
        invite: PathBuf,
        #[arg(short, long, default_value = "group.json")]
        out: PathBuf,
    },
    /// Print this member's Sender-Key distribution (send to peers out-of-band
    /// via `phantom send` so they can decrypt your future group messages).
    Distribution {
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
        #[arg(short, long, default_value = "group.json")]
        group_file: PathBuf,
        /// If set, write the distribution to this path instead of stdout (base64)
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Ingest a peer's distribution (so we can decrypt messages they encrypt)
    Accept {
        #[arg(short, long, default_value = "group.json")]
        group_file: PathBuf,
        /// Path to a file containing the base64 distribution produced by `distribution`
        #[arg(short, long)]
        dist: PathBuf,
    },
    /// Rotate your own Sender-Key chain (ratchet forward). Emits a new distribution.
    Rotate {
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
        #[arg(short, long, default_value = "group.json")]
        group_file: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Encrypt a plaintext message; emits group-wire bytes (base64 to stdout)
    Encrypt {
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
        #[arg(short, long, default_value = "group.json")]
        group_file: PathBuf,
        #[arg(short, long)]
        message: String,
    },
    /// Decrypt a base64-encoded group-wire payload
    Decrypt {
        #[arg(short, long, default_value = "group.json")]
        group_file: PathBuf,
        /// Base64 wire bytes (as produced by `encrypt`)
        #[arg(short = 'w', long)]
        wire: String,
    },
    /// Add a new member to the group (you still need to exchange distributions)
    Add {
        #[arg(short, long, default_value = "group.json")]
        group_file: PathBuf,
        /// Address of the new member in `view_hex:spend_hex` format
        #[arg(short, long)]
        member: String,
    },
    /// Remove a member. Rotates your own chain so they can't decrypt future messages.
    Remove {
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
        #[arg(short, long, default_value = "group.json")]
        group_file: PathBuf,
        /// Address of the member to remove
        #[arg(short, long)]
        member: String,
    },
    /// Print group metadata (group id, members, short-ids)
    Info {
        #[arg(short, long, default_value = "group.json")]
        group_file: PathBuf,
    },
}

// ─── helpers ──────────────────────────────────────────────────────────────────

fn load_signing(keyfile: &PathBuf) -> Result<PhantomSigningKey> {
    let json: serde_json::Value = serde_json::from_slice(
        &fs::read(keyfile).with_context(|| format!("reading {}", keyfile.display()))?,
    )?;
    let b64 = json
        .get("identity_private")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("identity_private missing from {}", keyfile.display()))?;
    let raw = B64
        .decode(b64)
        .context("identity_private is not valid base64")?;
    let arr: [u8; 32] = raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("identity_private must be 32 bytes"))?;
    Ok(PhantomSigningKey::from_bytes(arr))
}

/// Persist to disk via bincode (JSON can't encode `HashMap<[u8;8], _>` keys).
fn load_group(gf: &PathBuf) -> Result<PhantomGroup> {
    let bytes = fs::read(gf).with_context(|| format!("reading {}", gf.display()))?;
    bincode::deserialize(&bytes).context("deserialising group state (bincode)")
}

fn save_group(gf: &PathBuf, g: &PhantomGroup) -> Result<()> {
    let bytes = bincode::serialize(g).context("serialising group state (bincode)")?;
    fs::write(gf, bytes).with_context(|| format!("writing {}", gf.display()))?;
    Ok(())
}

fn parse_addr(s: &str) -> Result<PhantomAddress> {
    PhantomAddress::parse(s)
        .ok_or_else(|| anyhow!("invalid address '{}'; expected view_hex:spend_hex", s))
}

fn parse_members_csv(s: &str) -> Result<Vec<PhantomAddress>> {
    s.split(',')
        .map(|x| x.trim())
        .filter(|x| !x.is_empty())
        .map(parse_addr)
        .collect()
}

fn ok_banner(label: &str) {
    println!("{} {}", "✓".green().bold(), label.bold());
}

// ─── command handlers ────────────────────────────────────────────────────────

pub fn run(action: GroupAction) -> Result<()> {
    match action {
        GroupAction::Create { file, members, out, invite_out } => cmd_create(file, members, out, invite_out),
        GroupAction::Join { file, invite, out } => cmd_join(file, invite, out),
        GroupAction::Distribution { file, group_file, out } => cmd_distribution(file, group_file, out),
        GroupAction::Accept { group_file, dist } => cmd_accept(group_file, dist),
        GroupAction::Rotate { file, group_file, out } => cmd_rotate(file, group_file, out),
        GroupAction::Encrypt { file, group_file, message } => cmd_encrypt(file, group_file, message),
        GroupAction::Decrypt { group_file, wire } => cmd_decrypt(group_file, wire),
        GroupAction::Add { group_file, member } => cmd_add(group_file, member),
        GroupAction::Remove { file, group_file, member } => cmd_remove(file, group_file, member),
        GroupAction::Info { group_file } => cmd_info(group_file),
    }
}

fn cmd_create(file: PathBuf, members: String, out: PathBuf, invite_out: PathBuf) -> Result<()> {
    let signing = load_signing(&file)?;
    let members = parse_members_csv(&members)?;
    if members.is_empty() {
        return Err(anyhow!("at least one member required"));
    }
    let group = PhantomGroup::new(members.clone(), &signing);
    save_group(&out, &group)?;

    // Build invite blob from creator's group_id + roster
    let invite = GroupInvite {
        group_id: group.group_id,
        members,
    };
    let encoded = B64.encode(bincode::serialize(&invite).context("serialising invite")?);
    fs::write(&invite_out, encoded.as_bytes())?;

    ok_banner(&format!("group created — {} members → {}", group.members.len(), out.display()));
    ok_banner(&format!("invite blob → {} (share with other members)", invite_out.display()));
    println!("  each other member then runs: `phantom group join --invite {}`", invite_out.display());
    println!("  after join, exchange `phantom group distribution` outputs via `phantom group accept`");
    Ok(())
}

fn cmd_join(file: PathBuf, invite_path: PathBuf, out: PathBuf) -> Result<()> {
    let signing = load_signing(&file)?;
    let encoded = fs::read_to_string(&invite_path).with_context(|| format!("reading {}", invite_path.display()))?;
    let raw = B64.decode(encoded.trim()).context("invite is not valid base64")?;
    let invite: GroupInvite = bincode::deserialize(&raw).context("deserialising invite")?;

    // Build our local group state with OUR random own_chain, then overwrite
    // the randomly-assigned group_id with the creator's so the group matches.
    let mut group = PhantomGroup::new(invite.members.clone(), &signing);
    group.group_id = invite.group_id;
    save_group(&out, &group)?;

    ok_banner(&format!(
        "joined group {} ({} members) → {}",
        hex::encode(&group.group_id[..6]),
        group.members.len(),
        out.display()
    ));
    println!("  next: `phantom group distribution` and share with every other member via `accept`");
    Ok(())
}

fn cmd_distribution(file: PathBuf, group_file: PathBuf, out: Option<PathBuf>) -> Result<()> {
    let signing = load_signing(&file)?;
    let group = load_group(&group_file)?;
    let dist = group.own_distribution(&signing);
    let encoded = B64.encode(serde_json::to_vec(&dist)?);
    if let Some(p) = out {
        fs::write(&p, encoded.as_bytes())?;
        ok_banner(&format!("distribution → {}", p.display()));
    } else {
        println!("{}", encoded);
    }
    Ok(())
}

fn cmd_accept(group_file: PathBuf, dist_path: PathBuf) -> Result<()> {
    let mut group = load_group(&group_file)?;
    let encoded = fs::read_to_string(&dist_path).context("reading distribution file")?;
    let bytes = B64
        .decode(encoded.trim())
        .context("distribution file is not valid base64")?;
    let dist: SenderKeyDistribution =
        serde_json::from_slice(&bytes).context("deserialising SenderKeyDistribution")?;
    group.accept_distribution(dist);
    save_group(&group_file, &group)?;
    ok_banner("distribution registered — decrypt now possible for this sender");
    Ok(())
}

fn cmd_rotate(file: PathBuf, group_file: PathBuf, out: Option<PathBuf>) -> Result<()> {
    let signing = load_signing(&file)?;
    let mut group = load_group(&group_file)?;
    let dist = group.rotate_own_chain(&signing);
    save_group(&group_file, &group)?;
    let encoded = B64.encode(serde_json::to_vec(&dist)?);
    if let Some(p) = out {
        fs::write(&p, encoded.as_bytes())?;
        ok_banner(&format!("chain rotated — new distribution → {}", p.display()));
    } else {
        ok_banner("chain rotated — new distribution:");
        println!("{}", encoded);
    }
    println!("  distribute the new key to all current members");
    Ok(())
}

fn cmd_encrypt(file: PathBuf, group_file: PathBuf, message: String) -> Result<()> {
    let signing = load_signing(&file)?;
    let mut group = load_group(&group_file)?;
    let wire = group.encrypt(&signing, message.as_bytes());
    save_group(&group_file, &group)?;
    println!("{}", B64.encode(&wire));
    eprintln!("{} {} bytes on the wire", "✓".green(), wire.len());
    Ok(())
}

fn cmd_decrypt(group_file: PathBuf, wire: String) -> Result<()> {
    let mut group = load_group(&group_file)?;
    let bytes = B64.decode(wire.trim()).context("wire is not valid base64")?;
    match group.decrypt(&bytes) {
        Ok(plaintext) => {
            let txt = String::from_utf8_lossy(&plaintext);
            ok_banner("decrypted:");
            println!("{}", txt);
            save_group(&group_file, &group)?;
            Ok(())
        }
        Err(GroupError::UnknownSender) => Err(anyhow!(
            "unknown sender — they haven't sent a distribution yet (`phantom group accept ...`)"
        )),
        Err(e) => Err(anyhow!("decrypt failed: {:?}", e)),
    }
}

fn cmd_add(group_file: PathBuf, member_str: String) -> Result<()> {
    let mut group = load_group(&group_file)?;
    let member = parse_addr(&member_str)?;
    group.add_member(member);
    save_group(&group_file, &group)?;
    ok_banner(&format!("member added — now {} total", group.members.len()));
    println!("  send them your distribution; they need to send theirs back");
    Ok(())
}

fn cmd_remove(file: PathBuf, group_file: PathBuf, member_str: String) -> Result<()> {
    let signing = load_signing(&file)?;
    let mut group = load_group(&group_file)?;
    let member = parse_addr(&member_str)?;
    let before = group.members.len();
    let new_dist = group.remove_member(&member, &signing);
    save_group(&group_file, &group)?;
    let encoded = B64.encode(serde_json::to_vec(&new_dist)?);
    ok_banner(&format!(
        "removed 1 of {} — chain rotated, distribute the new key to remaining {} member(s):",
        before,
        group.members.len()
    ));
    println!("{}", encoded);
    Ok(())
}

fn cmd_info(group_file: PathBuf) -> Result<()> {
    let group = load_group(&group_file)?;
    println!("{}", "═ GROUP STATE ═".cyan().bold());
    println!("  group_id:   {}", hex::encode(group.group_id));
    println!("  members:    {}", group.members.len());
    for (i, m) in group.members.iter().enumerate() {
        let view = hex::encode(m.view_pub);
        let spend = hex::encode(m.spend_pub);
        println!(
            "    [{}] view={}… spend={}… short_id={}",
            i,
            &view[..12],
            &spend[..12],
            m.short_id()
        );
    }
    Ok(())
}
