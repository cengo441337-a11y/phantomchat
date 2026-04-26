// Wire types — match the Rust `Contact` / `IncomingMessage` / `Mls*` structs in
// `desktop/src-tauri/src/lib.rs`. Keep these in sync if you change either side.

export interface Contact {
  label: string;
  address: string;
  /// Hex-encoded Ed25519 public key bound to this contact for sealed-sender
  /// attribution. `null`/missing means "not bound yet" — the UI should show a
  /// small badge so the user remembers to bind.
  signing_pub?: string | null;
}

/// Payload pushed by the backend `message` event AND the on-disk shape
/// stored by `save_history`/`load_history`. Mirrors the Rust
/// `IncomingMessage` struct in `desktop/src-tauri/src/lib.rs`.
export interface IncomingMessage {
  plaintext: string;
  timestamp: string;
  /// Resolved sender label:
  ///   - contact name        — bound + good signature
  ///   - "you"               — outgoing row (we sent it)
  ///   - "INBOX"             — no attribution at all
  ///   - "INBOX!"            — attribution present but signature TAMPERED
  ///   - "?<8-hex>"          — good signature but pubkey not bound to a contact
  sender_label: string;
  sig_ok: boolean;
  sender_pub_hex?: string | null;
  /// Direction marker for persistence: "incoming" | "outgoing" | "system".
  /// Live `message` events from the backend always have "incoming"; the
  /// frontend tags outgoing/system rows itself before calling save_history.
  /// Optional because the backend defaults it via serde when absent.
  direction?: "incoming" | "outgoing" | "system";
  /// Row kind. `"text"` (default for backwards-compat), `"file"`, or
  /// `"voice"` (Wave 11B). The corresponding `file_meta` / `voice_meta`
  /// field is populated iff `kind` matches.
  kind?: "text" | "file" | "voice";
  /// File-row metadata. Only present for `kind === "file"` rows; mirrored
  /// 1:1 with the Rust `FileMeta` struct.
  file_meta?: FileMeta;
  /// Voice-row metadata. Only present for `kind === "voice"` rows
  /// (Wave 11B); mirrored 1:1 with the Rust `VoiceMeta` struct. Carries
  /// codec hint + duration_ms + on-disk path so the `<VoiceMessageBubble>`
  /// can render a duration label and feed the path through
  /// `convertFileSrc` for the `<audio>` element.
  voice_meta?: VoiceMeta;
  /// Stable per-message identifier used by the receipt path. Computed by
  /// the backend (sha256 over timestamp + plaintext, truncated to 16 hex).
  /// Present on outgoing rows (sender stamps it before send) and on
  /// incoming rows (receiver derives the same id at decode time).
  msg_id?: string;
  /// Outgoing-row delivery state. Escalates monotonically:
  ///   "sent"      -> single grey check
  ///   "delivered" -> double grey check
  ///   "read"      -> double cyber-cyan check
  /// Only meaningful on outgoing rows; incoming rows leave it undefined.
  delivery_state?: "sent" | "delivered" | "read";
  /// Per-message "pinned" flag. Persisted on the row itself so a reload
  /// preserves user intent. Default `false` is omitted from the on-disk
  /// JSON for backwards-compat with pre-feature persisted rows.
  pinned?: boolean;
  /// Per-message "starred" / favourite flag. Same persistence + back-
  /// compat strategy as `pinned`.
  starred?: boolean;
  /// Reply-thread metadata, populated when this row was sent via the
  /// REPL-1: envelope. The MessageStream renders an inline magenta
  /// "↪ <quoted_preview>" header that scrolls to the quoted row on click.
  reply_to?: ReplyMeta;
  /// Accumulated reactions for this row. Mutated in-place by the
  /// `reaction_updated` event so the UI can group + count emoji pills.
  reactions?: ReactionEntry[];
  /// Unix-epoch second after which this row should be hidden + dropped
  /// from on-disk history by the auto-purge sweep. Both endpoints stamp
  /// independently from their local TTL setting at send/receive time.
  expires_at?: number;
}

/// Per-message reply metadata mirrored from the Rust `ReplyMeta` struct.
/// `quoted_preview` is the first ~80 chars of the quoted message body so
/// the recipient can render the quote block without a history lookup.
export interface ReplyMeta {
  in_reply_to_msg_id: string;
  quoted_preview: string;
}

/// One emoji-reaction entry on a message row. Aggregated client-side
/// from the `RACT-1:` event stream.
export interface ReactionEntry {
  sender_label: string;
  emoji: string;
}

export type MsgKind = "incoming" | "outgoing" | "system";

/// In-app message line. Persisted to disk via `save_history` / `load_history`.
/// Backend's on-disk shape is `MessageRecord` — we keep TS identical so JSON
/// round-trips with no transform.
export interface MsgLine {
  ts: string;
  kind: MsgKind;
  label: string;
  body: string;
  /// `true` for verified attribution (or no-attribution case), `false` only
  /// when the sender forged a signature ("INBOX!" rows). System/outgoing
  /// rows leave this `true`. Optional + default-true for backwards compat
  /// with old persisted history files.
  sig_ok?: boolean;
  /// Hex-encoded Ed25519 sender pubkey, present for incoming sealed-sender
  /// rows. Used for the "bind unknown sender" flow.
  sender_pub_hex?: string | null;
  /// `"text"` (default — plain chat row), `"file"` (📎 attachment row),
  /// or `"voice"` (Wave 11B 🎙️ recording row). Optional so old persisted
  /// rows still round-trip cleanly.
  row_kind?: "text" | "file" | "voice";
  /// Populated for `row_kind === "file"` rows.
  file_meta?: FileMeta;
  /// Populated for `row_kind === "voice"` rows.
  voice_meta?: VoiceMeta;
  /// Stable per-message identifier (16 hex chars). Stamped on outgoing
  /// rows by the backend at send time and on incoming rows at decode
  /// time so receipts can match back to their originating row.
  msg_id?: string;
  /// Outgoing-row delivery state. The reducer in `App.tsx` escalates
  /// this monotonically as `receipt` events arrive (sent → delivered →
  /// read). Undefined / absent on incoming + system rows.
  delivery_state?: "sent" | "delivered" | "read";
  /// Pin / star flags — mirror the backend `IncomingMessage` fields.
  /// Toggled via the hover-toolbar's [Pin] / [Star] buttons, which fire
  /// the matching `pin_message` / `star_message` Tauri commands and
  /// listen for `message_state_changed` events.
  pinned?: boolean;
  starred?: boolean;
  /// Reply-thread metadata. Populated for rows that quote an earlier
  /// message; the stream renders the quote block above the body and
  /// scrolls-to-quoted on click.
  reply_to?: ReplyMeta;
  /// Per-row aggregated emoji reactions. Mutated by the
  /// `reaction_updated` event listener.
  reactions?: ReactionEntry[];
  /// Unix-epoch-second deadline for disappearing-messages auto-purge.
  /// Rows past this deadline are hidden in the UI and removed by the
  /// 60s backend purge sweep.
  expires_at?: number;
}

/// Per-file metadata. Mirrors the Rust `FileMeta` struct.
export interface FileMeta {
  filename: string;
  size: number;
  /// Absolute filesystem path on the receiver's disk where the bytes were
  /// saved. `null` for outgoing-side rows (the sender doesn't keep a copy
  /// under the PhantomChat Downloads dir).
  saved_path?: string | null;
  sha256_hex: string;
  /// `true` if the receiver re-hashed the bytes and matched. `false` for
  /// a tampered transfer. `undefined` for outgoing rows (no verify needed).
  sha256_ok?: boolean;
  /// MIME guess from the wire manifest (e.g. `image/png`). Drives the
  /// inline-image-vs-📎-link branch in `MessageStream.tsx`. Optional —
  /// legacy persisted rows pre-feature don't carry it.
  mime?: string;
}

/// Per-voice-message metadata. Mirrors the Rust `VoiceMeta` struct
/// (Wave 11B). The desktop saves the audio bytes under
/// `<app_data>/voice/<msg_id>.<ext>` and emits this struct inline on the
/// `IncomingMessage` so the `<VoiceMessageBubble>` can render duration
/// + feed the path through `convertFileSrc`.
export interface VoiceMeta {
  /// Container hint copied from the wire `codec_id`. `"opus"` (`.ogg`)
  /// or `"aac"` (`.m4a`). Both are HTML5 `<audio>`-decodable on every
  /// Tauri-supported platform without an extra Rust dep.
  codec: "opus" | "aac" | string;
  /// Duration of the recording in milliseconds. Set by the SENDER so the
  /// receiver can render `0:12` immediately, before the `<audio>`
  /// element's `loadedmetadata` event fires.
  duration_ms: number;
  /// Absolute filesystem path the desktop saved the audio bytes to.
  /// Pipe through `@tauri-apps/api/core::convertFileSrc` to get a
  /// `tauri://` URL the `<audio>` element can fetch.
  path: string;
}

/// Backend `file_received` event payload. Mirrors the Rust struct of the
/// same name. Emitted on every successful FILE1:01 envelope decode.
export interface FileReceivedEvent {
  from_label: string;
  filename: string;
  size: number;
  saved_path: string;
  sha256_ok: boolean;
  sha256_hex: string;
  ts: string;
  sender_pub_hex?: string | null;
  /// MIME hint copied from the wire manifest. Lets MessageStream branch
  /// to inline image-rendering for `image/*` payloads without inspecting
  /// the filename extension on the JS side.
  mime?: string;
}

/// Result returned by the `send_file` Tauri command. Used by the frontend
/// to immediately echo a "📎 sent <filename>" outgoing row.
export interface FileSendResult {
  filename: string;
  size: number;
  sha256_hex: string;
  mime?: string;
}

/// Relay/listener connection state for the StatusFooter pill. Emitted by the
/// backend on the `connection` event.
export type ConnectionStatus = "connecting" | "connected" | "disconnected";

export interface ConnectionEvent {
  status: ConnectionStatus;
  /// Optional human-readable detail (subscribe error message, etc.).
  detail?: string | null;
}

/// Per-message delivery / read receipt event emitted by the backend
/// listener whenever a peer's `RCPT-1:` envelope decodes. Mirrors the
/// Rust `ReceiptEvent` struct in `desktop/src-tauri/src/lib.rs`.
///
/// `App.tsx` listens for `receipt`, locates the outgoing row whose
/// `msg_id` matches, and escalates its `delivery_state` (never
/// downgrades — if a row is already "read" we ignore a late "delivered").
export interface ReceiptEvent {
  from_label: string;
  msg_id: string;
  kind: "delivered" | "read";
}

/// Typing-indicator event emitted by the backend listener whenever a
/// peer's `TYPN-1:` envelope decodes. Mirrors the Rust `TypingEvent`
/// struct. The frontend stores per-contact `expiry_ms` and renders the
/// "<label> is typing…" pill above the input bar until the deadline.
export interface TypingEvent {
  from_label: string;
  ttl_secs: number;
}

// ── MLS / Channels wire types ────────────────────────────────────────────────

export interface MlsAddResult {
  commit_b64: string;
  welcome_b64: string;
}

export interface MlsDecryptResult {
  plaintext: string | null;
  control_only: boolean;
}

export interface MlsStatus {
  initialized: boolean;
  in_group: boolean;
  member_count: number;
  identity_label: string | null;
  /// Auto-transport directory entries known to the bundle. Empty until
  /// `mls_add_member` runs (or the file is rehydrated from disk).
  members: MlsMemberRef[];
}

/// Per-MLS-member transport pointer — mirrors the Rust `MlsMemberRef`
/// struct, used by `mls_add_member` and decoded by the frontend so the
/// directory view can render labels + truncated addresses.
export interface MlsMemberRef {
  label: string;
  address: string;
  signing_pub_hex: string;
}

export interface MlsLogLine {
  ts: string;
  kind: "incoming" | "outgoing" | "system";
  body: string;
}

/// One row returned by the `mls_list_members` Tauri command. Mirrors
/// the Rust `MlsMemberInfo` struct in `desktop/src-tauri/src/lib.rs`.
export interface MlsMemberInfo {
  credential_label: string;
  signature_pub_hex: string;
  is_self: boolean;
  /// Label of the matching `MlsMemberRef` in the bundle's transport
  /// directory. `null` for the self-row or for a member that joined
  /// before our directory entry for them was cached.
  mapped_contact_label?: string | null;
}

/// Privacy-mode DTO mirrored from the Rust `PrivacyConfigDto`. Used by
/// `get_privacy_config` / `set_privacy_config` and by the Settings panel's
/// Privacy section. String-tagged enums match the JSON the backend reads
/// from `privacy.json` byte-for-byte.
export interface PrivacyConfigDto {
  mode: "DailyUse" | "MaximumStealth";
  proxy_addr: string;
  proxy_kind: "Tor" | "Nym";
}

// ── MLS auto-transport event payloads ───────────────────────────────────────
// Pushed by the backend listener when an incoming sealed-sender envelope
// carries the `MLS-WLC1` / `MLS-APP1` magic prefix (see lib.rs constants).
//
// `from_label` is resolved by `resolve_mls_from_label` against the bundle's
// `member_addresses` directory; unmatched senders surface as `?<8-hex>`
// just like the 1:1 path's unbound-sender placeholder.

export interface MlsJoinedEvent {
  from_label: string;
  group_member_count: number;
}

export interface MlsGroupMessage {
  from_label: string;
  plaintext: string;
  ts: string;
  member_count: number;
}

export interface MlsEpochEvent {
  member_count: number;
}

// ── Message search ──────────────────────────────────────────────────────────

/// One row returned by the `search_messages` Tauri command. Mirrors the
/// Rust `SearchHit` struct in `desktop/src-tauri/src/lib.rs`.
///
/// `match_ranges` is a list of `[start, end)` byte-offset pairs into
/// `plaintext` for each case-insensitive occurrence of the query. The
/// `SearchPanel` renders each range with a magenta background highlight.
/// An empty `match_ranges` array on a `kind === "file"` row indicates
/// the query matched the filename rather than the body — the panel
/// renders the row without per-character highlights in that case.
export interface SearchHit {
  msg_idx: number;
  timestamp: string;
  direction: string;
  sender_label: string;
  plaintext: string;
  kind?: "text" | "file" | string;
  match_ranges: Array<[number, number]>;
}

// ── Audit log (ISO27001 / ISMS append-only JSONL) ────────────────────────────
//
// Mirrors the Rust `AuditEntry` struct. `category` is one of
// "identity" | "contact" | "mls" | "relay" | "privacy" | "data" — kept as a
// loose `string` here so a backend extension doesn't force a TS recompile.
// `details` is an arbitrary JSON object whose shape varies per (category,
// event) pair.
export interface AuditEntry {
  ts: string;
  category: string;
  event: string;
  details: Record<string, unknown>;
}

// ── Crash reporting (Diagnostics) ────────────────────────────────────────────
//
// Mirrors the Rust `CrashReport` struct. Each row is one captured panic:
// timestamp, app version, OS, the panic's first-line message + source
// location, and the captured backtrace. `user_dispatched` flips to `true`
// after a successful POST via `dispatch_crash_report` so the UI can render
// "already sent" instead of offering a re-send.
export interface CrashReport {
  ts: string;
  version: string;
  os: string;
  panic_msg: string;
  location: string;
  backtrace: string;
  user_dispatched?: boolean;
}

// ── LAN org (mDNS zero-touch discovery) ─────────────────────────────────────
//
// Mirrors the Rust `LanOrgStatus` + `DiscoveredPeer` structs. The shared-
// secret "org code" is the only authentication for mDNS-discovered peers;
// they're auto-added as 1:1 contacts but NOT as MLS group members.

/// Returned by `lan_org_status`. `active` is true iff a `ServiceDaemon` is
/// running; `code` is the user-shareable 6-char `XXX-XXX` string;
/// `peer_count` is the deduplicated discovered-peer count;
/// `last_discovery_ts` is the unix-epoch-seconds string of the most-recent
/// resolve (or null if we've broadcast but never seen a peer).
export interface LanOrgStatus {
  active: boolean;
  code?: string | null;
  peer_count: number;
  last_discovery_ts?: string | null;
}

/// Pushed by the `lan_peer_discovered` event whenever the browse task
/// resolves a never-before-seen peer with a matching org code.
export interface DiscoveredPeer {
  label: string;
  address: string;
  signing_pub_hex: string;
  last_seen: number;
}

// ── Auto-updater wire types ──────────────────────────────────────────────────
//
// Mirrors the Rust `UpdateInfo` struct from the `check_for_updates` /
// `install_update` Tauri commands wrapping `tauri-plugin-updater`.
export interface UpdateInfo {
  available: boolean;
  version?: string | null;
  release_notes?: string | null;
}

// ── Wave 8G: Pin / Star (per-message) + Archive (per-conversation) ──────────
//
// Mirrors the Rust `ConversationState` struct in
// `desktop/src-tauri/src/lib.rs`. Persisted under `conversation_state.json`
// keyed by contact label.

export interface ConversationState {
  archived?: boolean;
  pinned?: boolean;
  muted?: boolean;
}

/// Emitted by `pin_message` / `unpin_message` / `star_message` /
/// `unstar_message` so the React reducer can patch the in-memory
/// `messages` array without reloading history.
export interface MessageStateChangedEvent {
  msg_id: string;
  pinned: boolean;
  starred: boolean;
}

/// Emitted by the conversation-level mutations
/// (`archive_conversation` / `unarchive_conversation` /
/// `pin_conversation` / `unpin_conversation`). The frontend hydrates
/// the contact-state map from `get_conversation_state` on cold start
/// and patches in-place from this event.
export interface ConversationStateChangedEvent {
  contact_label: string;
  state: ConversationState;
}

// ── Reply / reactions / disappearing-messages event payloads ────────────────
//
// All three are emitted by the Rust listener path that decodes the matching
// `REPL-1:` / `RACT-1:` / `DISA-1:` envelope. The reply event reuses the
// existing `message` channel (with `reply_to` populated) so React's reducer
// stays the same. The other two get dedicated channels.

/// Emitted on `reaction_updated` after the backend mutates a target row's
/// reactions array. `reactions` is the FULL post-update list so the React
/// reducer can patch in place without merging.
export interface ReactionUpdatedEvent {
  target_msg_id: string;
  reactions: ReactionEntry[];
}

/// Emitted on `messages_purged` by the 60s auto-purge sweep. `msg_ids`
/// is the list of rows the sweep removed from `messages.json` so the
/// React state can drop them in lockstep.
export interface MessagesPurgedEvent {
  msg_ids: string[];
}

/// Emitted on `disappearing_ttl_changed` whenever a peer's `DISA-1:`
/// envelope decodes (or our own `set_disappearing_ttl` succeeds locally).
/// `ttl_secs == null` disables auto-purge for the conversation.
export interface DisappearingTtlChangedEvent {
  contact_label: string;
  ttl_secs: number | null;
}

// ── Encrypted backup / restore (Wave 8c, compliance Aufbewahrungspflicht) ────
//
// Mirror the three Rust DTOs returned by `export_backup` / `verify_backup`
// / `import_backup`. Used by the Backup section of SettingsPanel to render
// the success toast (sha256 + path), the pre-restore provenance preview,
// and the post-restore item count.

export interface BackupResult {
  path: string;
  size_bytes: number;
  sha256_hex: string;
  item_count: number;
}

export interface BackupMeta {
  version: number;
  created_at: string;
  item_count: number;
  host_label: string;
}

export interface RestoreResult {
  items_restored: number;
  identity_replaced: boolean;
  requires_restart: boolean;
}

// ── Wave 11A: Home-LLM Bridge ──────────────────────────────────────────────
//
// Mirrors `desktop/src-tauri/src/ai_bridge.rs`. The four providers correspond
// to four ways a Pro/Team-subscriber or pay-as-you-go user can plug an LLM
// into PhantomChat without breaking the messenger's E2E story:
//   - `ollama`        — local HTTP, nothing leaves the box
//   - `claude_cli`    — `claude --print` subprocess, OAuth via Claude Code
//                       (counts against the user's Pro/Team plan)
//   - `openai_compat` — any /v1/chat/completions endpoint with a bearer key
//   - `claude_api`    — Anthropic's native /v1/messages with `x-api-key`

export type AiBridgeProvider =
  | "ollama"
  | "claude_cli"
  | "openai_compat"
  | "claude_api";

export interface AiBridgeConfig {
  active: boolean;
  provider: AiBridgeProvider;

  ollama_endpoint: string;
  ollama_model: string;

  claude_cli_path: string;
  claude_cli_extra_args: string[];

  openai_endpoint: string;
  openai_api_key: string;
  openai_model: string;

  claude_api_key: string;
  claude_api_model: string;

  system_prompt: string;
  allowlist: string[];
  max_history_turns: number;
}

export interface AiBridgeTurn {
  role: "user" | "assistant";
  content: string;
}
