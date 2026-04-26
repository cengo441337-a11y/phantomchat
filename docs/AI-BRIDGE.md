# PhantomChat — Home-LLM Bridge (Wave 11)

> _Send a message from your phone, your home machine answers via Claude / Ollama / OpenAI. End-to-end encrypted. No VPN, no port-forward, no cloud middleman besides the LLM itself._

The AI Bridge is what turns PhantomChat from "encrypted messenger" into
"your personal AI agent that you can reach from anywhere on the planet,
running on hardware you own".

---

## What it is

PhantomChat Desktop runs on your home machine as a daemon. When a message
arrives from a contact you have explicitly allow-listed, the daemon:

1. Decrypts the sealed-sender envelope (existing E2E + double-ratchet path)
2. Pipes the plaintext into a local or hosted LLM
3. Sends the LLM's response back through the same E2E + relay path

From the **mobile** side this looks like a regular contact you chat with.
The transport is the existing PhantomChat E2E + Nostr-relay pipeline
(sealed-sender, signed-attribution, optional Tor-mode, optional
Dandelion++) — no new auth surface, no new server.

---

## Why it's interesting

| Use case | Without AI Bridge | With AI Bridge |
|----------|-------------------|----------------|
| Ask my home-Claude to deploy a PR while I'm on the train | VPN, SSH, terminal | Send a chat message |
| Get a long-running build / scrape / data-pipeline result | SSH back later, hope tmux didn't die | Home-Claude pushes the answer when ready |
| Voice-control a workflow on your home rig from your phone | Doesn't exist | Tap-and-hold mic → speak → Claude answers |
| Continue a Claude Code session you started yesterday | Open laptop | Talk to your home-Claude through the phone |
| Use your Pro/Team subscription on mobile without paying API metered rates | Pay metered or wait | ClaudeCli provider counts against your existing plan |
| Keep all the prompt + answer plaintext out of vendor logs | Cloud-only chat apps record everything | Ollama provider keeps the entire pipeline local |

The big architectural unlock: **PhantomChat's E2E + relay layer doubles
as the most paranoid-grade tunnel you'll ever ship for talking to your
home machine**. A LAN-discoverable, NAT-traversable, Tor-routable,
metadata-blind tunnel — without you setting any of that up.

---

## Provider matrix

| Provider | Auth | Cost | Privacy | Tools |
|----------|------|------|---------|-------|
| `ClaudeCli` (default) | `claude login` (Claude Code OAuth, lives in `~/.claude/`) | counts vs your Pro / Team plan | per Anthropic data policy | YES — full Claude Code tool stack (Bash, Read, Edit, MCP) |
| `Ollama` | none | free | strongest — never leaves the box | depends on model |
| `OpenAiCompat` | API key (Bearer) | metered | varies by endpoint | model-dependent |
| `ClaudeApi` | Anthropic API key (`x-api-key`) | metered (pay-per-token) | per Anthropic API ToS | none (unless you wire tools yourself) |

`ClaudeCli` is the default for one reason: if you already pay for Claude
Pro or Team, it routes through that subscription's quota with zero
extra spend, and you inherit every tool / MCP server / setting that
your local Claude Code already has.

---

## Setup — `ClaudeCli` provider (the recommended path)

On your home machine:

1. Install Claude Code: `curl -LsSf https://claude.ai/install.sh | sh`
2. Run `claude login` once — opens a browser for the OAuth flow,
   writes tokens into `~/.claude/`.
3. Open PhantomChat Desktop → Settings → AI Bridge.
4. Provider: **Claude CLI (your Pro/Team subscription)** (default).
5. Path to `claude` CLI: leave as `claude` if it's on PATH, otherwise
   absolute path (e.g. `D:\rust\.cargo\bin\claude.exe`).
6. Extra args: optional. Examples:
   - `--model claude-sonnet-4-6` (cheaper / faster than the Opus
     default)
   - `--mcp-config ~/.config/claude/mcp.json` if you have a custom
     MCP-server set
7. **Tool-Permissions automatisch genehmigen**: leave on. A headless
   bridge can't answer interactive prompts. With this off, Claude
   stalls forever the first time it wants to invoke Bash.
8. Allow-list: comma-separated contact labels that are allowed to talk
   to the bridge. **Don't put unattributed senders on this list** —
   the code already rejects `INBOX` / `INBOX!` / `?<hex>` defensively.
9. Toggle "Enable AI Bridge".
10. Click "Test provider" — should round-trip a "pong" or similar.

You're done. From your phone, send a message to your home identity.
Claude reads it, optionally invokes tools (`gh pr list`, `cargo build`,
your custom MCP server), and replies.

## Setup — `Ollama` provider (privacy-maximalist path)

For users who want the entire pipeline local — no API calls, no
subscription, plaintext never leaves the home machine.

1. Install Ollama: <https://ollama.com/download>
2. Pull a model: `ollama pull llama3.1` (or `mistral`, `qwen2.5`,
   etc. — pick one that fits your RAM budget).
3. Settings → AI Bridge → Provider: **Ollama (local, free)**.
4. Endpoint: `http://localhost:11434` (default).
5. Model: matches whatever you pulled (`llama3.1`).
6. Allow-list, toggle, test — same as above.

Tradeoff: Ollama models don't natively use tools. The bridge
forwards your message and gets a text answer back; no Bash, no MCP.
For "ask my home-Claude to deploy a thing", use `ClaudeCli`. For
"my home-AI is a private journal I can dump thoughts into without any
provider seeing them", use Ollama.

## Setup — API-key providers

`OpenAiCompat` works with any endpoint speaking `/v1/chat/completions`
with bearer auth — Groq, Together, Mistral, OpenRouter, vLLM, etc.

`ClaudeApi` hits Anthropic's native `/v1/messages` with `x-api-key`.
Use this if you specifically don't want a Claude Code subscription
and prefer pay-as-you-go.

---

## Conversation memory

Per-contact rolling history is persisted in
`<app_data>/ai_bridge_history.json` and capped at
`max_history_turns` (default 10 user-assistant pairs). A daemon
restart does not wipe context. Setting `max_history_turns: 0` makes
every reply stateless — useful when the bridge sits in front of a
tool-using agent that handles its own state via MCP / tmux / etc.

To wipe a single contact's history without touching the rest:

```ts
await invoke("ai_bridge_clear_history", { contactLabel: "alice" });
```

---

## Security model

- **The bridge only auto-replies to allow-listed contacts.** Unattributed
  senders (`INBOX`, `INBOX!`, `?<hex>` — i.e. messages from people whose
  signing pubkey hasn't been bound to a contact label) never trigger
  a reply, even if the bridge is active. Belt-and-suspenders: if your
  allow-list ever has a typo, the worst that happens is the bridge
  doesn't reply, not that it replies to a stranger.

- **Allow-listed contacts can invoke tools.** When using `ClaudeCli`
  with `claude_cli_skip_permissions` enabled, an allow-listed contact
  who sends "delete everything in /home/deniz" gets that exact
  behavior. Don't allow-list contacts you don't trust to operate your
  machine. This is the same trust level as giving them a shell.

  **Default since 3.0.2:** `claude_cli_skip_permissions` defaults to
  `false`. Tool-approval prompts are surfaced via the existing
  conversation history (the bridge will reply with the prompt text and
  expect a confirming follow-up message). Flip the toggle on
  consciously if you want headless full-trust operation.

- **Voice messages now feed into the LLM via on-device STT.** Wave 11D
  added the whisper.cpp transcription step described in
  [Voice-message integration § Wave 11D](#wave-11d--voice--stt--llm).
  Audio bytes never leave the desktop; only the transcribed text is
  forwarded to the configured LLM provider.

- **API keys are stored in plaintext** in `<app_data>/ai_bridge.json`.
  They're protected by the OS file-permissions of that directory. If
  this matters in your threat model, use the `ClaudeCli` provider
  (no key in PhantomChat config — Claude Code handles the OAuth
  refresh in `~/.claude/`).

- **Audit-log entries** at `<app_data>/audit.log`:
  - `ai_bridge.config_set` — every save, scrubbed of secrets
  - `ai_bridge.replied` — sender + char counts (no plaintext)
  - `ai_bridge.send_failed` / `provider_error` — error string only

---

## Voice-message integration

### Wave 11B — receive only

Voice messages were saved to `<app_data>/voice/<msg_id>.<ext>` and
surfaced as `kind: "voice"` rows in the chat, but the bridge did NOT
auto-reply to them — the receive handler short-circuited on the
`VOICE-1:` wire prefix.

### Wave 11D — voice → STT → LLM

The bridge now closes the loop. When an inbound message hits the
listener, the routing is:

```
VOICE-1:<header><opus-or-aac>
        │
        ├─→ handle_incoming_voice_v1
        │     · saves audio to <app_data>/voice/<msg_id>.<ext>
        │     · emits IncomingMessage{kind:"voice"} to the React side
        │
        └─→ ai_bridge_maybe_handle (Wave 11D)
              · checks should_respond(sender_label)
              · runs whisper.cpp on the saved file (BLOCKING in spawn_blocking)
              · feeds the transcript into ai_bridge::complete(...) like a typed turn
              · sends the LLM reply back through the standard send pipeline
```

The audio bytes never leave the desktop machine. Even with a
cloud-LLM provider configured (Claude, OpenAI), the LLM only ever sees
the transcribed TEXT — same privacy property as the rest of the bridge.

#### Setup

1. Open Settings → AI Bridge → "Voice → Text → LLM (Wave 11D)".
2. Click "Download model" next to your preferred model. Files are
   pulled from
   `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-<name>.bin`
   into `<app_data>/whisper/`. Progress bar updates in real time.
3. Pick the downloaded model from the dropdown.
4. (Optional) Pick a language. Auto-detect works fine for most cases
   but adds ~100 ms of overhead for the first 30 s of every clip.
5. Tick "Enable STT for voice messages".

#### Model selection — tradeoffs

| Model | Size | Languages | Speed (CPU) | WER  | Recommended for |
|-------|------|-----------|-------------|------|------------------|
| `tiny.en` | 39 MB | English only | fastest | high | English-only smoke tests |
| `tiny` | 75 MB | multilingual | fastest | high | low-RAM boxes, all-language smoke tests |
| `base.en` | 74 MB | English only | very fast | medium | English-only daily use on weak CPU |
| `base` | 142 MB | multilingual | very fast | medium | **default recommendation** — German+English on commodity hardware |
| `small.en` | 244 MB | English only | fast | low | English voice-to-action, accuracy-sensitive |
| `small` | 466 MB | multilingual | fast | low | multi-language daily use |
| `medium.en` | 769 MB | English only | medium | very low | English-only, accuracy >> latency |
| `medium` | 1.5 GB | multilingual | medium | very low | best-in-class for multi-language voice |

`.en` variants are roughly half the size and 5-15 % faster than the
multilingual peers, at the cost of being English-only. If your home
contacts speak any non-English language at all, pick the multilingual
variant.

#### Compile-time gating

STT is behind a Cargo feature `stt` (default-on). The whisper-rs
build pulls in `whisper.cpp` via `cmake` + a C compiler — CI runners
without that toolchain can build with `--no-default-features` and ship
a working bridge that just skips voice messages (same as Wave 11B
behaviour). The Tauri commands `ai_bridge_list_whisper_models` and
`ai_bridge_download_whisper_model` stay registered in either build so
the Settings UI renders consistently.

#### Privacy guarantee

- Audio bytes are decoded + transcribed entirely in-process on the
  desktop. No network calls during STT.
- The LLM provider receives only the transcribed text, which it then
  treats as a normal user turn (history, system prompt, allow-list — all
  the existing Wave 11A/F gates apply unchanged).
- The decoded PCM is never written to disk; only the original
  encoded bytes (already on disk from the Wave 11B receive path) and
  the resulting text exist in persistent storage.

---

## Wave 11E — Proactive Watchers (shipped)

Until Wave 11E the bridge was strictly reactive: it only replied to
incoming messages from allow-listed contacts. Wave 11E adds **proactive
pushes** — the home daemon sends UNPROMPTED messages on triggers (CI
green, scheduled task, file changed, etc.) to a pre-configured target
contact.

### Use-case examples

| Goal | Schedule | Mode | Command |
|------|----------|------|---------|
| Ping me when CI passes | every 60 s | `AlertOnly` (inverted: send iff exit ≠ 0; for "green" use a script that exits non-zero on red) | `gh run list --workflow=ci.yml --limit 1 --json conclusion -q '.[0].conclusion' \| grep -q success` |
| Hourly deploy-status push | cron `0 0 * * * *` (every hour at :00:00) | `Raw` | `kubectl rollout status deploy/web --timeout=10s` |
| 9 AM daily overnight log summary | cron `0 0 9 * * *` | `Summarize` | `journalctl --since=yesterday \| grep -i error` |
| File-change watcher (poll) | every 30 s | `Summarize` | `find ~/Downloads/incoming -mmin -1 -type f` |
| Hourly disk-space alert | cron `0 0 * * * *` | `AlertOnly` | `df -h / \| awk 'NR==2 {if (substr($5,1,length($5)-1)+0 > 90) exit 1}'` |

`Raw` sends the command's stdout verbatim, prefixed with `🔔 [<watcher_name>]\n`.
`Summarize` runs the stdout through the configured LLM with a "Summarize
in 1-3 sentences" system prompt. `AlertOnly` only sends when the exit
code is non-zero — useful for "no news is good news" scenarios.

### Cron-expression cheatsheet

The `cron` crate expects a **6-field** expression: `sec min hour
day-of-month month day-of-week` (with an optional 7th `year` field). This
differs from classic Unix cron's 5-field form — leave `sec` as `0` to
match what crontab.guru shows.

| Pattern | Meaning |
|---------|---------|
| `0 * * * * *` | every minute, on the minute |
| `0 */5 * * * *` | every 5 minutes |
| `0 0 * * * *` | every hour, on the hour |
| `0 0 9 * * *` | every day at 9:00 AM |
| `0 0 9 * * Mon-Fri` | weekdays at 9:00 AM |
| `0 30 18 * * Fri` | every Friday at 6:30 PM |
| `0 0 0 1 * *` | midnight on the 1st of each month |

The Settings UI ships a "Verify" button next to the cron input that
calls back into the Rust `cron::Schedule::from_str` parser and renders
the next-fire timestamp inline. Reference: <https://crontab.guru>
(translate the 5-field pattern from there into 6-field by prefixing `0 `).

### Security model

Watchers run shell commands AS THE BRIDGE PROCESS USER. Anyone who can
edit `<app_data>/ai_bridge_watchers.json` (or who can persuade the user
to add a watcher via the Settings panel) can execute arbitrary code on
the home machine — same threat class as `claude_cli_skip_permissions`.

The mitigations:

- **Allow-list gate at send time.** A watcher whose `target_contact` is
  not on the AI-Bridge allow-list logs `error: target '...' not on
  AI-bridge allowlist` and skips the send. Defends against typo'd
  labels or a malicious config silently exfiltrating output to an
  attacker-controlled contact.
- **Audit-log entries.** Every `add` / `update` / `remove` / `fired` /
  `failed` event lands in `audit.log` under category `ai_bridge`,
  events `watcher_added` / `watcher_updated` / `watcher_removed` /
  `watcher_fired` / `watcher_failed`. Compliance auditors can grep on
  category=ai_bridge to see the full lifecycle.
- **Per-command timeout.** A 5-minute wall-clock cap kills runaway
  commands so a hung process doesn't hold a watcher slot forever.
- **stdout truncation.** Output exceeding 8000 chars is truncated
  before send, defending the chat from being spammed by megabyte-scale
  output.

### Known limitations / punted items

- **Per-watcher concurrency lock — shipped in 3.0.2.** A `tokio::sync::Mutex`
  per watcher-id now prevents a second invocation while the first is still
  running; multi-click on "Run now" no longer fans out.
- **No retry on transient send failures.** A watcher whose send fails
  (relay down, target offline, etc.) records `error: send failed: ...`
  in `last_status` and waits for the next tick. The chat is the source
  of truth, so the lost push is gone.
- **Cron timezone is `Local`.** Watchers fire in the host's local
  timezone, not UTC. Set `TZ=` in the bridge process environment if
  you need a different anchor.

---

## Roadmap

- **Wave 11G — mobile-auto-update half: shipped.** The Android client
  now polls a signed manifest URL and surfaces in-app update banners.
  The watcher-side concurrency-lock half landed in 3.0.2 (above).
- **Wave 11H — future** — pre-built watcher templates: "GitHub Actions
  watcher", "filesystem-change watcher", "Slack overnight summary
  watcher" — one-click install with the appropriate command + schedule
  + mode pre-filled. Reduces the "I need to know shell" barrier.
- **Per-watcher fire-history view in Settings — future** — currently
  you grep `audit.log` for `category=ai_bridge event=watcher_fired`.
