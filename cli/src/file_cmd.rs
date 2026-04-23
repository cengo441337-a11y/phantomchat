//! `phantom file` subcommand — encrypt a file for a group, decrypt an archive.

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use clap::Subcommand;
use colored::Colorize;
use phantomchat_core::{
    file_transfer::{pack, peek_header, unpack_into, DEFAULT_CHUNK_SIZE},
    group::PhantomGroup,
    keys::PhantomSigningKey,
};
use std::{fs, path::PathBuf};

#[derive(Subcommand)]
pub enum FileAction {
    /// Encrypt a file into a .ptf archive addressed to a group
    Pack {
        /// Identity file produced by `phantom keygen`
        #[arg(short, long, default_value = "keys.json")]
        file: PathBuf,
        /// Group state (produced by `phantom group create`/`join`)
        #[arg(short = 'g', long, default_value = "group.json")]
        group_file: PathBuf,
        /// Path to plaintext input file
        #[arg(short, long)]
        input: PathBuf,
        /// Output archive path (`.ptf` recommended)
        #[arg(short, long)]
        out: PathBuf,
        /// Chunk size (bytes). Smaller = more overhead, tighter length masking.
        #[arg(long, default_value_t = DEFAULT_CHUNK_SIZE)]
        chunk_size: usize,
        /// Override stored filename in the manifest (default: input basename)
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Decrypt a .ptf archive, validate SHA256, write plaintext to disk
    Unpack {
        #[arg(short = 'g', long, default_value = "group.json")]
        group_file: PathBuf,
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Inspect an archive header without decrypting (safe pre-check)
    Info {
        #[arg(short, long)]
        input: PathBuf,
    },
}

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

fn load_group(gf: &PathBuf) -> Result<PhantomGroup> {
    let bytes = fs::read(gf).with_context(|| format!("reading {}", gf.display()))?;
    bincode::deserialize(&bytes).context("deserialising group state (bincode)")
}

fn save_group(gf: &PathBuf, g: &PhantomGroup) -> Result<()> {
    let bytes = bincode::serialize(g).context("serialising group state (bincode)")?;
    fs::write(gf, bytes).with_context(|| format!("writing {}", gf.display()))?;
    Ok(())
}

fn human_bytes(n: u64) -> String {
    const K: f64 = 1024.0;
    let n = n as f64;
    if n < K { return format!("{n:.0} B"); }
    if n < K * K { return format!("{:.1} KiB", n / K); }
    if n < K * K * K { return format!("{:.1} MiB", n / (K * K)); }
    format!("{:.1} GiB", n / (K * K * K))
}

pub fn run(action: FileAction) -> Result<()> {
    match action {
        FileAction::Pack { file, group_file, input, out, chunk_size, name } => {
            cmd_pack(file, group_file, input, out, chunk_size, name)
        }
        FileAction::Unpack { group_file, input, out } => cmd_unpack(group_file, input, out),
        FileAction::Info { input } => cmd_info(input),
    }
}

fn cmd_pack(
    file: PathBuf,
    group_file: PathBuf,
    input: PathBuf,
    out: PathBuf,
    chunk_size: usize,
    name: Option<String>,
) -> Result<()> {
    let signing = load_signing(&file)?;
    let mut group = load_group(&group_file)?;
    let plaintext = fs::read(&input).with_context(|| format!("reading {}", input.display()))?;
    let filename = match name {
        Some(s) => s,
        None => input
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    };
    let archive = pack(&mut group, &signing, &filename, &plaintext, chunk_size)
        .map_err(|e| anyhow!("pack failed: {e}"))?;
    save_group(&group_file, &group)?;
    fs::write(&out, &archive).with_context(|| format!("writing {}", out.display()))?;
    println!(
        "{} {} → {}  ({} plaintext → {} archive, {} chunks)",
        "✓".green().bold(),
        input.display(),
        out.display(),
        human_bytes(plaintext.len() as u64),
        human_bytes(archive.len() as u64),
        plaintext.len().div_ceil(chunk_size.max(1)),
    );
    Ok(())
}

fn cmd_unpack(group_file: PathBuf, input: PathBuf, out: PathBuf) -> Result<()> {
    let mut group = load_group(&group_file)?;
    let archive = fs::read(&input).with_context(|| format!("reading {}", input.display()))?;
    let (hdr, plain) = unpack_into(&mut group, &archive)
        .map_err(|e| anyhow!("unpack failed: {e}"))?;
    save_group(&group_file, &group)?;
    fs::write(&out, &plain).with_context(|| format!("writing {}", out.display()))?;
    println!(
        "{} {} → {}  ({}, {} chunks, sha256 ok)",
        "✓".green().bold(),
        input.display(),
        out.display(),
        human_bytes(hdr.total_size),
        hdr.chunk_count,
    );
    if !hdr.filename.is_empty() && hdr.filename != out.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() {
        println!("  {} archive's stored filename was: {}", "note:".dimmed(), hdr.filename);
    }
    Ok(())
}

fn cmd_info(input: PathBuf) -> Result<()> {
    let archive = fs::read(&input).with_context(|| format!("reading {}", input.display()))?;
    let hdr = peek_header(&archive).map_err(|e| anyhow!("bad archive: {e}"))?;
    println!("{}", "═ ARCHIVE HEADER ═".cyan().bold());
    println!("  group_id:    {}", hex::encode(hdr.group_id));
    println!("  total_size:  {} ({} bytes)", human_bytes(hdr.total_size), hdr.total_size);
    println!("  sha256:      {}", hex::encode(hdr.sha256));
    println!("  chunk_count: {}", hdr.chunk_count);
    println!("  filename:    {}", if hdr.filename.is_empty() { "(none)".to_string() } else { hdr.filename });
    println!("  archive:     {}", human_bytes(archive.len() as u64));
    Ok(())
}
