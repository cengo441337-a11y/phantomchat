//! Wave 11A — Home-LLM Bridge.
//!
//! Lets the user run PhantomChat Desktop as a daemon that auto-responds to
//! incoming messages from explicitly allow-listed contacts by routing the
//! plaintext through a local or hosted LLM. The mobile client treats the
//! bridge identity as a regular contact — no special UI, just sends a
//! message and gets a reply through the same E2E + relay pipeline.
//!
//! ## Provider matrix
//!
//! | Provider      | Auth                | Cost                | Privacy posture          |
//! |---------------|--------------------|--------------------|---------------------------|
//! | `Ollama`      | none (local HTTP)  | free                | strongest — nothing leaves the box |
//! | `ClaudeCli`   | `claude login`     | counts vs Pro/Team plan | depends on Anthropic data policy |
//! | `OpenAiCompat`| API key            | metered             | varies by endpoint                 |
//! | `ClaudeApi`   | API key            | metered             | per Anthropic API ToS              |
//!
//! `ClaudeCli` is the no-extra-cost path for Pro/Team subscribers — invokes
//! `claude --print "<prompt>"` as a subprocess so OAuth tokens stay in
//! `~/.claude/` (managed by Claude Code), never touched by PhantomChat.
//!
//! ## Conversation state
//!
//! Per-contact rolling history capped at `max_history_turns`. Persisted in
//! `app_data/ai_bridge_history.json` so a daemon restart does not lose
//! mid-conversation context. Setting `max_history_turns` to 0 makes every
//! reply stateless (useful when the bridge sits in front of a tool-using
//! agent that handles its own state).

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command as TokioCommand;

const CONFIG_FILE: &str = "ai_bridge.json";
const HISTORY_FILE: &str = "ai_bridge_history.json";
const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";
const DEFAULT_OLLAMA_MODEL: &str = "llama3.1";
const DEFAULT_CLAUDE_CLI_PATH: &str = "claude";
const DEFAULT_OPENAI_ENDPOINT: &str = "https://api.openai.com/v1";
const DEFAULT_OPENAI_MODEL: &str = "gpt-4o-mini";
const DEFAULT_CLAUDE_API_MODEL: &str = "claude-opus-4-7";
const DEFAULT_MAX_HISTORY_TURNS: u32 = 10;
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Ollama,
    ClaudeCli,
    OpenAiCompat,
    ClaudeApi,
}

impl Default for ProviderKind {
    fn default() -> Self {
        ProviderKind::ClaudeCli
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiBridgeConfig {
    pub active: bool,
    pub provider: ProviderKind,

    pub ollama_endpoint: String,
    pub ollama_model: String,

    pub claude_cli_path: String,
    /// Extra args appended after `--print`. Useful for `--model
    /// claude-sonnet-4-6`, `--mcp-config`, etc.
    pub claude_cli_extra_args: Vec<String>,
    /// When true, pass `--dangerously-skip-permissions` so Claude can
    /// invoke tools (Bash, Read, Edit, MCP servers) without an interactive
    /// approval prompt that would deadlock a headless bridge. Default
    /// true — the bridge runs unattended by definition. Disable only if
    /// you specifically want a no-tools "chat-only" bridge.
    #[serde(default = "default_true")]
    pub claude_cli_skip_permissions: bool,

    pub openai_endpoint: String,
    pub openai_api_key: String,
    pub openai_model: String,

    pub claude_api_key: String,
    pub claude_api_model: String,

    pub system_prompt: String,
    /// Contact labels permitted to invoke the bridge. Anyone else is ignored
    /// (no auto-reply, no logging beyond the standard inbox path).
    pub allowlist: Vec<String>,
    pub max_history_turns: u32,

    /// Per-contact overrides applied on top of the base config when the
    /// sender's label matches a key. Missing fields inherit from base.
    /// Use cases:
    ///   - route a senior contact to claude-opus; route a casual one to ollama
    ///   - give different contacts different system prompts (work vs personal)
    ///   - cap history at 0 for one contact (stateless) but 50 for another
    #[serde(default)]
    pub contact_overrides: HashMap<String, ContactOverride>,
}

/// Per-contact override applied on top of `AiBridgeConfig`. Every field is
/// optional — `None` means "use the base config's value". Stored in the
/// same `ai_bridge.json` so a single save round-trips everything.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContactOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderKind>,
    /// Override the per-provider model name. Routed to the right field
    /// based on the (possibly-overridden) provider:
    ///   ollama        → ollama_model
    ///   openai_compat → openai_model
    ///   claude_api    → claude_api_model
    ///   claude_cli    → appended as `--model <m>` to the CLI args
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_history_turns: Option<u32>,
}

fn default_true() -> bool {
    true
}

impl Default for AiBridgeConfig {
    fn default() -> Self {
        Self {
            active: false,
            provider: ProviderKind::default(),
            ollama_endpoint: DEFAULT_OLLAMA_ENDPOINT.to_string(),
            ollama_model: DEFAULT_OLLAMA_MODEL.to_string(),
            claude_cli_path: DEFAULT_CLAUDE_CLI_PATH.to_string(),
            claude_cli_extra_args: Vec::new(),
            claude_cli_skip_permissions: true,
            openai_endpoint: DEFAULT_OPENAI_ENDPOINT.to_string(),
            openai_api_key: String::new(),
            openai_model: DEFAULT_OPENAI_MODEL.to_string(),
            claude_api_key: String::new(),
            claude_api_model: DEFAULT_CLAUDE_API_MODEL.to_string(),
            system_prompt: "You are PhantomChat, the user's home assistant. The user is messaging you from their phone; keep replies concise and actionable.".to_string(),
            allowlist: Vec::new(),
            max_history_turns: DEFAULT_MAX_HISTORY_TURNS,
            contact_overrides: HashMap::new(),
        }
    }
}

/// Apply any per-contact override on top of the base config. Returned as
/// `Cow::Borrowed` when there's no override (zero-copy hot path) and
/// `Cow::Owned` when an override merge produced a fresh struct.
pub fn effective_config<'a>(
    cfg: &'a AiBridgeConfig,
    contact_label: &str,
) -> Cow<'a, AiBridgeConfig> {
    let Some(override_) = cfg.contact_overrides.get(contact_label) else {
        return Cow::Borrowed(cfg);
    };
    let mut effective = cfg.clone();
    if let Some(p) = override_.provider {
        effective.provider = p;
    }
    if let Some(m) = &override_.model {
        match effective.provider {
            ProviderKind::Ollama => effective.ollama_model = m.clone(),
            ProviderKind::OpenAiCompat => effective.openai_model = m.clone(),
            ProviderKind::ClaudeApi => effective.claude_api_model = m.clone(),
            ProviderKind::ClaudeCli => {
                // Inject `--model <m>` ahead of the user-provided extras so
                // the user can still append flags after this in the base
                // config (last-arg-wins is claude's behaviour).
                let mut new_args = vec!["--model".to_string(), m.clone()];
                new_args.extend(effective.claude_cli_extra_args.drain(..));
                effective.claude_cli_extra_args = new_args;
            }
        }
    }
    if let Some(sp) = &override_.system_prompt {
        effective.system_prompt = sp.clone();
    }
    if let Some(mht) = override_.max_history_turns {
        effective.max_history_turns = mht;
    }
    Cow::Owned(effective)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct HistoryDisk {
    /// `contact_label` → bounded ring of turns (oldest first).
    per_contact: HashMap<String, Vec<Turn>>,
}

pub fn config_path(app_data_dir: &std::path::Path) -> PathBuf {
    app_data_dir.join(CONFIG_FILE)
}

pub fn history_path(app_data_dir: &std::path::Path) -> PathBuf {
    app_data_dir.join(HISTORY_FILE)
}

pub fn load_config(app_data_dir: &std::path::Path) -> AiBridgeConfig {
    std::fs::read(config_path(app_data_dir))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_config(app_data_dir: &std::path::Path, cfg: &AiBridgeConfig) -> Result<()> {
    let path = config_path(app_data_dir);
    let buf =
        serde_json::to_vec_pretty(cfg).with_context(|| "serialising AiBridgeConfig")?;
    std::fs::write(&path, buf)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn load_history(app_data_dir: &std::path::Path) -> HistoryDisk {
    std::fs::read(history_path(app_data_dir))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_history(app_data_dir: &std::path::Path, disk: &HistoryDisk) -> Result<()> {
    let path = history_path(app_data_dir);
    let buf = serde_json::to_vec_pretty(disk).with_context(|| "serialising history")?;
    std::fs::write(&path, buf)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Append a turn to `contact_label`'s rolling history and persist. Truncates
/// to the most-recent `max_turns` entries (oldest dropped first).
pub fn append_turn(
    app_data_dir: &std::path::Path,
    contact_label: &str,
    turn: Turn,
    max_turns: u32,
) -> Result<()> {
    let mut disk = load_history(app_data_dir);
    let entry = disk
        .per_contact
        .entry(contact_label.to_string())
        .or_default();
    entry.push(turn);
    if max_turns > 0 && entry.len() > max_turns as usize {
        let drop = entry.len() - max_turns as usize;
        entry.drain(0..drop);
    }
    save_history(app_data_dir, &disk)
}

pub fn get_history(app_data_dir: &std::path::Path, contact_label: &str) -> Vec<Turn> {
    load_history(app_data_dir)
        .per_contact
        .get(contact_label)
        .cloned()
        .unwrap_or_default()
}

pub fn clear_history(app_data_dir: &std::path::Path, contact_label: &str) -> Result<()> {
    let mut disk = load_history(app_data_dir);
    disk.per_contact.remove(contact_label);
    save_history(app_data_dir, &disk)
}

/// Returns true iff the bridge is active AND `sender_label` is on the
/// allowlist AND not an unattributed sender (`INBOX`, `INBOX!`, `?<hex>`).
pub fn should_respond(cfg: &AiBridgeConfig, sender_label: &str) -> bool {
    if !cfg.active {
        return false;
    }
    if sender_label.starts_with('?') || sender_label == "INBOX" || sender_label == "INBOX!" {
        return false;
    }
    cfg.allowlist.iter().any(|l| l == sender_label)
}

/// Run a completion through the configured provider. Pure I/O, no PhantomChat
/// state — caller wires the response back via the existing send pipeline.
pub async fn complete(
    cfg: &AiBridgeConfig,
    history: &[Turn],
    user_message: &str,
) -> Result<String> {
    match cfg.provider {
        ProviderKind::Ollama => ollama_complete(cfg, history, user_message).await,
        ProviderKind::ClaudeCli => claude_cli_complete(cfg, history, user_message).await,
        ProviderKind::OpenAiCompat => openai_complete(cfg, history, user_message).await,
        ProviderKind::ClaudeApi => claude_api_complete(cfg, history, user_message).await,
    }
}

// ── Ollama ──────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OllamaChatRequest<'a> {
    model: &'a str,
    messages: Vec<OllamaMessage<'a>>,
    stream: bool,
}

#[derive(Serialize)]
struct OllamaMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

async fn ollama_complete(
    cfg: &AiBridgeConfig,
    history: &[Turn],
    user_message: &str,
) -> Result<String> {
    let mut messages = Vec::with_capacity(history.len() + 2);
    if !cfg.system_prompt.is_empty() {
        messages.push(OllamaMessage {
            role: "system",
            content: &cfg.system_prompt,
        });
    }
    for t in history {
        messages.push(OllamaMessage {
            role: match t.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            },
            content: &t.content,
        });
    }
    messages.push(OllamaMessage {
        role: "user",
        content: user_message,
    });

    let body = OllamaChatRequest {
        model: &cfg.ollama_model,
        messages,
        stream: false,
    };

    let url = format!("{}/api/chat", cfg.ollama_endpoint.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS))
        .build()
        .with_context(|| "building reqwest client")?;
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {}", url))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Ollama HTTP {}: {}", status, txt));
    }
    let parsed: OllamaChatResponse = resp
        .json()
        .await
        .with_context(|| "decoding Ollama response")?;
    Ok(parsed.message.content)
}

// ── Claude CLI subprocess ───────────────────────────────────────────────────
// `claude --print <prompt>` reads OAuth tokens from `~/.claude/` (managed by
// Claude Code) and counts against the user's Pro/Team subscription. We do
// NOT touch tokens or auth state — that's Claude Code's domain.

async fn claude_cli_complete(
    cfg: &AiBridgeConfig,
    history: &[Turn],
    user_message: &str,
) -> Result<String> {
    // Compose the prompt as a single string. The CLI does not expose a
    // multi-turn JSON ingest, so we serialise prior turns into the prompt
    // body. Format mirrors what most LLMs expect when given a flat prompt.
    let mut prompt = String::new();
    if !cfg.system_prompt.is_empty() {
        prompt.push_str("System: ");
        prompt.push_str(&cfg.system_prompt);
        prompt.push_str("\n\n");
    }
    for t in history {
        match t.role {
            Role::User => prompt.push_str("User: "),
            Role::Assistant => prompt.push_str("Assistant: "),
        }
        prompt.push_str(&t.content);
        prompt.push_str("\n\n");
    }
    prompt.push_str("User: ");
    prompt.push_str(user_message);

    let mut cmd = TokioCommand::new(&cfg.claude_cli_path);
    cmd.arg("--print");
    if cfg.claude_cli_skip_permissions {
        cmd.arg("--dangerously-skip-permissions");
    }
    for a in &cfg.claude_cli_extra_args {
        cmd.arg(a);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning '{}'", cfg.claude_cli_path))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .await
            .with_context(|| "writing prompt to claude stdin")?;
        // Drop closes stdin so claude knows the prompt is complete.
        drop(stdin);
    }

    let output = tokio::time::timeout(
        Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    .with_context(|| "claude CLI timeout")?
    .with_context(|| "waiting for claude CLI")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "claude CLI exit {}: {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Err(anyhow!("claude CLI produced empty output"));
    }
    Ok(stdout)
}

// ── OpenAI-compatible (also Groq, Together, Mistral, OpenRouter, etc.) ──────

#[derive(Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiMessage<'a>>,
}

#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Deserialize)]
struct OpenAiResponseMessage {
    content: String,
}

async fn openai_complete(
    cfg: &AiBridgeConfig,
    history: &[Turn],
    user_message: &str,
) -> Result<String> {
    if cfg.openai_api_key.is_empty() {
        return Err(anyhow!("openai_api_key not configured"));
    }
    let mut messages = Vec::with_capacity(history.len() + 2);
    if !cfg.system_prompt.is_empty() {
        messages.push(OpenAiMessage {
            role: "system",
            content: &cfg.system_prompt,
        });
    }
    for t in history {
        messages.push(OpenAiMessage {
            role: match t.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            },
            content: &t.content,
        });
    }
    messages.push(OpenAiMessage {
        role: "user",
        content: user_message,
    });

    let body = OpenAiChatRequest {
        model: &cfg.openai_model,
        messages,
    };

    let url = format!(
        "{}/chat/completions",
        cfg.openai_endpoint.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS))
        .build()
        .with_context(|| "building reqwest client")?;
    let resp = client
        .post(&url)
        .bearer_auth(&cfg.openai_api_key)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {}", url))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("OpenAI HTTP {}: {}", status, txt));
    }
    let parsed: OpenAiChatResponse = resp
        .json()
        .await
        .with_context(|| "decoding OpenAI response")?;
    parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| anyhow!("OpenAI response had zero choices"))
}

// ── Anthropic native /v1/messages ──────────────────────────────────────────

#[derive(Serialize)]
struct ClaudeApiRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<ClaudeApiMessage<'a>>,
}

#[derive(Serialize)]
struct ClaudeApiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ClaudeApiResponse {
    content: Vec<ClaudeApiContent>,
}

#[derive(Deserialize)]
struct ClaudeApiContent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

async fn claude_api_complete(
    cfg: &AiBridgeConfig,
    history: &[Turn],
    user_message: &str,
) -> Result<String> {
    if cfg.claude_api_key.is_empty() {
        return Err(anyhow!("claude_api_key not configured"));
    }
    let mut messages = Vec::with_capacity(history.len() + 1);
    for t in history {
        messages.push(ClaudeApiMessage {
            role: match t.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            },
            content: &t.content,
        });
    }
    messages.push(ClaudeApiMessage {
        role: "user",
        content: user_message,
    });

    let body = ClaudeApiRequest {
        model: &cfg.claude_api_model,
        max_tokens: 1024,
        system: &cfg.system_prompt,
        messages,
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS))
        .build()
        .with_context(|| "building reqwest client")?;
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &cfg.claude_api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .with_context(|| "POST https://api.anthropic.com/v1/messages")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Anthropic HTTP {}: {}", status, txt));
    }
    let parsed: ClaudeApiResponse = resp
        .json()
        .await
        .with_context(|| "decoding Anthropic response")?;
    let text = parsed
        .content
        .into_iter()
        .filter(|c| c.kind == "text")
        .map(|c| c.text)
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return Err(anyhow!("Anthropic response had no text content blocks"));
    }
    Ok(text)
}

