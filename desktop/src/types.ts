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
  /// Row kind. `"text"` (default for backwards-compat) or `"file"`. The
  /// `file_meta` field is populated iff `kind === "file"`.
  kind?: "text" | "file";
  /// File-row metadata. Only present for `kind === "file"` rows; mirrored
  /// 1:1 with the Rust `FileMeta` struct.
  file_meta?: FileMeta;
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
  /// `"text"` (default — plain chat row) or `"file"` (📎 attachment row).
  /// Optional so old persisted rows still round-trip cleanly.
  row_kind?: "text" | "file";
  /// Populated for `row_kind === "file"` rows.
  file_meta?: FileMeta;
  /// Stable per-message identifier (16 hex chars). Stamped on outgoing
  /// rows by the backend at send time and on incoming rows at decode
  /// time so receipts can match back to their originating row.
  msg_id?: string;
  /// Outgoing-row delivery state. The reducer in `App.tsx` escalates
  /// this monotonically as `receipt` events arrive (sent → delivered →
  /// read). Undefined / absent on incoming + system rows.
  delivery_state?: "sent" | "delivered" | "read";
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
}

/// Result returned by the `send_file` Tauri command. Used by the frontend
/// to immediately echo a "📎 sent <filename>" outgoing row.
export interface FileSendResult {
  filename: string;
  size: number;
  sha256_hex: string;
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

// ── Auto-updater wire types ──────────────────────────────────────────────────
//
// Mirrors the Rust `UpdateInfo` struct from the `check_for_updates` /
// `install_update` Tauri commands wrapping `tauri-plugin-updater`.
export interface UpdateInfo {
  available: boolean;
  version?: string | null;
  release_notes?: string | null;
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
