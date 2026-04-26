import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type {
  ConnectionEvent,
  ConnectionStatus,
  Contact,
  ConversationState,
  ConversationStateChangedEvent,
  FileReceivedEvent,
  FileSendResult,
  IncomingMessage,
  MessageStateChangedEvent,
  DisappearingTtlChangedEvent,
  FileReceivedEvent,
  FileSendResult,
  IncomingMessage,
  MessagesPurgedEvent,
  MlsEpochEvent,
  MlsGroupMessage,
  MlsJoinedEvent,
  MlsLogLine,
  MlsStatus,
  MsgLine,
  ReactionUpdatedEvent,
  ReceiptEvent,
  TypingEvent,
  UpdateInfo,
} from "./types";
import ContactsPane from "./components/ContactsPane";
import ChannelsPane from "./components/ChannelsPane";
import ConversationHeader from "./components/ConversationHeader";
import MessageStream from "./components/MessageStream";
import InputBar from "./components/InputBar";
import StatusFooter from "./components/StatusFooter";
import AddContactModal from "./components/AddContactModal";
import BindContactModal from "./components/BindContactModal";
import IdentityGate from "./components/IdentityGate";
import SettingsPanel from "./components/SettingsPanel";
import OnboardingWizard from "./components/OnboardingWizard";
import SearchPanel from "./components/SearchPanel";
import { plaintextMentions } from "./lib/mentions";

type LeftPaneTab = "contacts" | "channels";

const DEFAULT_RELAY = "wss://relay.damus.io";
const HISTORY_DEBOUNCE_MS = 500;

function nowHHMMSS(): string {
  return new Date().toLocaleTimeString("en-GB", { hour12: false });
}

/// Translate a UI MsgLine back to the backend `IncomingMessage` shape. Used
/// for `save_history`. The Rust side accepts the extra `direction` field;
/// older fields default cleanly thanks to `#[serde(default)]`.
function msgLineToWire(m: MsgLine): IncomingMessage {
  const direction =
    m.kind === "incoming"
      ? "incoming"
      : m.kind === "outgoing"
      ? "outgoing"
      : "system";
  return {
    plaintext: m.body,
    timestamp: m.ts,
    sender_label: m.label,
    sig_ok: m.sig_ok ?? true,
    sender_pub_hex: m.sender_pub_hex ?? null,
    direction,
    kind: m.row_kind ?? "text",
    file_meta: m.file_meta,
    msg_id: m.msg_id,
    delivery_state: m.delivery_state,
    pinned: m.pinned ?? false,
    starred: m.starred ?? false,
    reply_to: m.reply_to,
    reactions: m.reactions,
    expires_at: m.expires_at,
  };
}

function wireToMsgLine(m: IncomingMessage): MsgLine {
  const kind: MsgLine["kind"] =
    m.direction === "outgoing"
      ? "outgoing"
      : m.direction === "system"
      ? "system"
      : "incoming";
  return {
    ts: m.timestamp,
    kind,
    label: m.sender_label,
    body: m.plaintext,
    sig_ok: m.sig_ok,
    sender_pub_hex: m.sender_pub_hex ?? null,
    row_kind: m.kind ?? "text",
    file_meta: m.file_meta,
    msg_id: m.msg_id,
    delivery_state: m.delivery_state,
    pinned: m.pinned ?? false,
    starred: m.starred ?? false,
    reply_to: m.reply_to,
    reactions: m.reactions,
    expires_at: m.expires_at,
  };
}

/// 1024-base humanizer for size labels — mirrors the backend `human_size`
/// helper so sender-echo and listener-event rows render with identical
/// suffixes ("12.4 KiB" / "3.2 MiB").
export function humanSize(n: number): string {
  const K = 1024;
  if (n < K) return `${n} B`;
  if (n < K * K) return `${(n / K).toFixed(1)} KiB`;
  if (n < K * K * K) return `${(n / (K * K)).toFixed(1)} MiB`;
  return `${(n / (K * K * K)).toFixed(1)} GiB`;
}

export default function App() {
  const { t } = useTranslation();

  // ── Identity / boot state ──────────────────────────────────────────────
  const [address, setAddress] = useState<string | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);
  /// Local user's `me.json` label, used as the canonical "self" name
  /// for the mention auto-complete + the loud `notify_mention` trigger.
  /// Empty string until the boot probe resolves.
  const [myLabel, setMyLabel] = useState<string>("");

  // ── Auto-updater banner ───────────────────────────────────────────────
  // Set on the cold-start `check_for_updates` round-trip if the endpoint
  // reports a newer release. Surfaces a passive banner at the top of the
  // window pointing the user at Settings → About.
  const [updateAvailable, setUpdateAvailable] = useState<UpdateInfo | null>(
    null,
  );

  // ── First-launch wizard ────────────────────────────────────────────────
  // `null` while we're still asking the backend; `true`/`false` once the
  // `is_onboarded` round-trip resolves. We hold rendering until we know,
  // so we don't briefly flash the main UI before the wizard takes over.
  const [isOnboarded, setIsOnboarded] = useState<boolean | null>(null);

  // ── Settings overlay ───────────────────────────────────────────────────
  const [showSettings, setShowSettings] = useState(false);

  // ── Search panel state ─────────────────────────────────────────────────
  // `showSearch` toggles the slide-down search bar (Ctrl/Cmd+F, Esc).
  // `searchHighlightIdx` is the message-array index that the panel asked
  // to jump to — MessageStream scroll-into-views + pulses on it. Setting
  // it back to undefined cancels the highlight (e.g. when the panel
  // reopens fresh).
  const [showSearch, setShowSearch] = useState(false);
  const [searchHighlightIdx, setSearchHighlightIdx] = useState<
    number | undefined
  >(undefined);

  // ── Contacts + messaging state ─────────────────────────────────────────
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [activeLabel, setActiveLabel] = useState<string | null>(null);
  const [messages, setMessages] = useState<MsgLine[]>([]);
  /// Wave 8G — `conversation_state.json` map, keyed by contact label.
  /// Hydrated on cold start via `get_conversation_state`, then patched
  /// in-place from `conversation_state_changed` events. Drives the
  /// archive/pin layout in ContactsPane + the SettingsPanel archive view.
  const [conversationState, setConversationState] = useState<
    Record<string, ConversationState>
  >({});
  const [showAddContact, setShowAddContact] = useState(false);
  const [showBindModal, setShowBindModal] = useState(false);
  /// Hex pubkey of the most-recent unbound sealed-sender, mirroring
  /// backend `AppState.last_unbound_sender`. We track it here purely for
  /// the banner / modal — actual binding is a backend round-trip.
  const [pendingUnboundPub, setPendingUnboundPub] = useState<string | null>(
    null,
  );

  // ── Footer counters ────────────────────────────────────────────────────
  const [scanned, setScanned] = useState(0);
  const [decrypted, setDecrypted] = useState(0);
  const [connection, setConnection] = useState<ConnectionStatus>("connecting");

  // ── Reply-to compose state ─────────────────────────────────────────────
  // Set when the user clicks Reply on a row in MessageStream. InputBar
  // reads it to render the quote block above the input + route sends
  // through `send_reply` instead of `send_message`. Cleared after the
  // send resolves (or on cancel).
  const [replyingTo, setReplyingTo] = useState<{
    msg_id: string;
    preview: string;
  } | null>(null);

  // ── Typing indicators ──────────────────────────────────────────────────
  // Map<contact_label, expiry_ms>. Set on every `typing` event from the
  // backend; entries auto-expire via `setTimeout`. The InputBar reads the
  // active contact's entry to render the "<label> is typing…" pill.
  const [typingUntil, setTypingUntil] = useState<Map<string, number>>(
    () => new Map(),
  );

  // ── Left-pane tab (1:1 contacts vs MLS channels) ───────────────────────
  const [leftTab, setLeftTab] = useState<LeftPaneTab>("contacts");

  // ── MLS state lifted out of ChannelsPane ───────────────────────────────
  // Events (`mls_joined`, `mls_message`, `mls_epoch`) keep flowing into the
  // log + trigger status refreshes even when ChannelsPane is unmounted —
  // tab-switching back to channels shows the full transcript instead of a
  // blank panel.
  const [mlsLog, setMlsLog] = useState<MlsLogLine[]>([]);
  const [mlsStatus, setMlsStatus] = useState<MlsStatus | null>(null);

  const pushMlsLog = useCallback((kind: MlsLogLine["kind"], body: string) => {
    setMlsLog(l => [...l, { ts: nowHHMMSS(), kind, body }]);
  }, []);

  const refreshMlsStatus = useCallback(async () => {
    try {
      const s = await invoke<MlsStatus>("mls_status");
      setMlsStatus(s);
    } catch (e) {
      pushMlsLog("system", t("channels_pane.log.status_error", { error: String(e) }));
    }
  }, [pushMlsLog, t]);

  // Avoid double-spinning the listener under React.StrictMode dev double-mount.
  const listenerStarted = useRef(false);
  // Skip the initial save that would fire immediately after `load_history`
  // hydrates `messages`. Otherwise we'd overwrite the disk file with the
  // same contents on every cold start (harmless, but wasteful).
  const historyHydrated = useRef(false);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Hold a stable ref to the latest `messages` so the debounced save always
  // serializes the freshest state instead of the closed-over snapshot.
  const messagesRef = useRef<MsgLine[]>([]);
  messagesRef.current = messages;

  // ── Push helpers ───────────────────────────────────────────────────────
  function pushSystem(body: string) {
    setMessages(m => [
      ...m,
      { ts: nowHHMMSS(), kind: "system", label: "·", body, sig_ok: true },
    ]);
  }
  function pushIncoming(payload: IncomingMessage) {
    setMessages(m => [
      ...m,
      {
        ts: payload.timestamp ?? nowHHMMSS(),
        kind: "incoming",
        label: payload.sender_label,
        body: payload.plaintext,
        sig_ok: payload.sig_ok,
        sender_pub_hex: payload.sender_pub_hex ?? null,
        // Stash the msg_id the backend computed at decode time so the
        // IntersectionObserver in MessageStream can pass it back through
        // `mark_read` to ack the read state.
        msg_id: payload.msg_id,
        // REPL-1: rows carry the inline quote metadata; reactions hydrate
        // from the listener's RACT-1: stream and disappear-rows carry
        // their pre-computed `expires_at`.
        reply_to: payload.reply_to,
        reactions: payload.reactions,
        expires_at: payload.expires_at,
      },
    ]);
  }
  function pushOutgoing(
    label: string,
    body: string,
    msgId?: string,
    opts?: {
      reply_to?: { in_reply_to_msg_id: string; quoted_preview: string };
      expires_at?: number;
    },
  ) {
    setMessages(m => [
      ...m,
      {
        ts: nowHHMMSS(),
        kind: "outgoing",
        label,
        body,
        sig_ok: true,
        // The send path stamps the same `msg_id` the backend will derive
        // on the receiver side, so subsequent receipt events can locate
        // this row. Initial state is "sent" — escalates on `receipt`.
        msg_id: msgId,
        delivery_state: msgId ? "sent" : undefined,
        reply_to: opts?.reply_to,
        expires_at: opts?.expires_at,
      },
    ]);
  }
  /// Append an incoming-side file row from the backend `file_received`
  /// event. `body` doubles as the user-visible label so legacy text-only
  /// renderers still produce something meaningful — the file row component
  /// keys off `row_kind` first.
  function pushFileIncoming(payload: FileReceivedEvent) {
    setMessages(m => [
      ...m,
      {
        ts: payload.ts,
        kind: "incoming",
        label: payload.from_label,
        body: `received ${payload.filename} (${humanSize(payload.size)})`,
        sig_ok: true,
        sender_pub_hex: payload.sender_pub_hex ?? null,
        row_kind: "file",
        file_meta: {
          filename: payload.filename,
          size: payload.size,
          saved_path: payload.saved_path,
          sha256_hex: payload.sha256_hex,
          sha256_ok: payload.sha256_ok,
          mime: payload.mime,
        },
      },
    ]);
  }
  /// Append an outgoing-side file row immediately after `send_file` resolves.
  function pushFileOutgoing(label: string, result: FileSendResult) {
    setMessages(m => [
      ...m,
      {
        ts: nowHHMMSS(),
        kind: "outgoing",
        label,
        body: `sent ${result.filename} (${humanSize(result.size)})`,
        sig_ok: true,
        row_kind: "file",
        file_meta: {
          filename: result.filename,
          size: result.size,
          sha256_hex: result.sha256_hex,
          // Sender doesn't keep a copy under PhantomChat/, so leave path
          // unset; the row's "open folder" affordance is hidden when path
          // is missing.
          saved_path: null,
          mime: result.mime,
        },
      },
    ]);
  }

  // ── Debounced history save ─────────────────────────────────────────────
  // Re-arm a 500ms timer on every state change; only when the timer fires
  // do we actually `save_history` the current snapshot. Avoids hammering
  // the disk on rapid bursts (e.g. multi-line paste, MLS welcome flood).
  useEffect(() => {
    if (!historyHydrated.current) return;
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      const wire = messagesRef.current.map(msgLineToWire);
      invoke("save_history", { messages: wire }).catch(e => {
        // Swallow — a transient save failure shouldn't pop a system message
        // every keystroke. The next save attempt will retry.
        console.warn("save_history failed:", e);
      });
    }, HISTORY_DEBOUNCE_MS);
    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
    };
  }, [messages]);

  // ── Probe first-launch wizard marker ───────────────────────────────────
  // This runs before anything else so the wizard takes precedence over
  // IdentityGate / the main UI. A backend error defaults to "onboarded"
  // so a broken `is_onboarded` command can't hard-lock the user out.
  useEffect(() => {
    (async () => {
      try {
        const ok = await invoke<boolean>("is_onboarded");
        setIsOnboarded(ok);
      } catch {
        setIsOnboarded(true);
      }
    })();
  }, []);

  // ── Cold-start auto-update check ───────────────────────────────────────
  // Fire `check_for_updates` once on mount. If the configured endpoint says
  // a newer release is out, surface the passive banner. Failures are silent
  // — a transient network issue (or unreachable update server) shouldn't
  // produce a UI error on every launch. The user can manually retry from
  // Settings → About.
  useEffect(() => {
    (async () => {
      try {
        const info = await invoke<UpdateInfo>("check_for_updates");
        if (info.available) setUpdateAvailable(info);
      } catch {
        /* silent — see comment above */
      }
    })();
  }, []);

  // ── Boot: load history + get_address + list_contacts + start_listener ──
  useEffect(() => {
    // Don't kick off boot until we know the wizard isn't blocking the UI;
    // running listener / address probes during onboarding would race with
    // the wizard's `generate_identity` / `set_relays` calls.
    if (isOnboarded !== true) return;
    (async () => {
      // Hydrate persisted history first so the user sees their last session
      // immediately, before any address probe / listener spin-up.
      try {
        const persisted = await invoke<IncomingMessage[]>("load_history");
        if (persisted.length > 0) {
          setMessages(persisted.map(wireToMsgLine));
        }
      } catch (e) {
        console.warn("load_history failed:", e);
      } finally {
        historyHydrated.current = true;
      }

      try {
        const addr = await invoke<string>("get_address");
        setAddress(addr);
      } catch (e) {
        // No identity yet — IdentityGate will prompt the user to generate.
        setAddress(null);
        return;
      }
      // Pull the persisted self-label so the mention pipeline can render
      // an extra-prominent pill (and trigger `notify_mention`) when a
      // peer addresses us by name. Empty string is fine — it short-
      // circuits the local-mention check inside `plaintextMentions`.
      try {
        const lbl = await invoke<string>("get_my_label");
        setMyLabel(lbl ?? "");
      } catch {
        /* not fatal */
      }
      try {
        const cs = await invoke<Contact[]>("list_contacts");
        setContacts(cs);
        if (cs.length > 0) setActiveLabel(cs[0].label);
      } catch (e) {
        pushSystem(t("app.system.load_contacts_failed", { error: String(e) }));
      }

      // Wave 8G — hydrate per-conversation pin/archive map. Errors are
      // silent + fall back to an empty map so a missing/malformed
      // `conversation_state.json` doesn't block the chat UI.
      try {
        const cs = await invoke<Record<string, ConversationState>>(
          "get_conversation_state",
        );
        setConversationState(cs ?? {});
      } catch (e) {
        console.warn("get_conversation_state failed:", e);
      }

      // Pick up any cached connection status the backend already knows.
      try {
        const cs = await invoke<string>("get_connection_status");
        if (cs === "connected" || cs === "disconnected" || cs === "connecting") {
          setConnection(cs);
        }
      } catch {
        /* not fatal */
      }

      if (!listenerStarted.current) {
        listenerStarted.current = true;
        try {
          const status = await invoke<string>("start_listener", {
            relayUrl: DEFAULT_RELAY,
          });
          pushSystem(status);
        } catch (e) {
          pushSystem(t("app.system.listener_failed", { error: String(e) }));
          setBootError(String(e));
        }
      }
    })();
  }, [isOnboarded]);

  // ── Subscribe to backend events ────────────────────────────────────────
  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];

    listen<IncomingMessage>("message", e => {
      pushIncoming(e.payload);
      setScanned(s => s + 1);
      setDecrypted(d => d + 1);

      // Surface the unbound-pending state so ContactsPane can render the
      // banner. Pubkey hex is the canonical "what's pending" identifier.
      if (
        e.payload.sender_label.startsWith("?") &&
        e.payload.sender_pub_hex
      ) {
        setPendingUnboundPub(e.payload.sender_pub_hex);
      }
    }).then(u => unlisteners.push(u));

    listen<null>("scanned", () => {
      setScanned(s => s + 1);
    }).then(u => unlisteners.push(u));

    listen<string>("error", e => {
      pushSystem(t("app.system.error_prefix", { message: e.payload }));
    }).then(u => unlisteners.push(u));

    listen<string>("status", e => {
      pushSystem(e.payload);
    }).then(u => unlisteners.push(u));

    listen<ConnectionEvent>("connection", e => {
      setConnection(e.payload.status);
      if (e.payload.status === "disconnected" && e.payload.detail) {
        pushSystem(t("app.system.relay_disconnected", { detail: e.payload.detail }));
      }
    }).then(u => unlisteners.push(u));

    // ── MLS auto-transport events ────────────────────────────────────────
    // These fire from the backend listener whenever an incoming sealed-
    // sender envelope carries an `MLS-WLC1` / `MLS-APP1` magic prefix.
    // They go straight into the lifted `mlsLog` so ChannelsPane sees them
    // even when its tab is hidden.
    listen<MlsJoinedEvent>("mls_joined", e => {
      const { from_label, group_member_count } = e.payload;
      pushMlsLog(
        "system",
        `joined group from ${from_label} · ${group_member_count} member${group_member_count === 1 ? "" : "s"}`,
      );
      void refreshMlsStatus();
    }).then(u => unlisteners.push(u));

    listen<MlsGroupMessage>("mls_message", e => {
      const { from_label, plaintext } = e.payload;
      pushMlsLog("incoming", `${from_label}: ${plaintext}`);
      // Mention check: if the message names US (by `me.label`), fire the
      // loud `notify_mention` so the user sees the system-shelf entry
      // even if the window is focused. We use the latest `myLabel` via
      // closure — it's set at boot and only changes via Settings.
      if (myLabel && from_label !== myLabel) {
        const hits = plaintextMentions(plaintext, [myLabel]);
        if (hits.length > 0) {
          void invoke("notify_mention", {
            fromLabel: from_label,
            body: plaintext,
          }).catch(err => console.warn("notify_mention failed:", err));
        }
      }
    }).then(u => unlisteners.push(u));

    listen<MlsEpochEvent>("mls_epoch", e => {
      pushMlsLog(
        "system",
        `epoch advanced · ${e.payload.member_count} member${e.payload.member_count === 1 ? "" : "s"}`,
      );
      void refreshMlsStatus();
    }).then(u => unlisteners.push(u));

    // ── File transfer: incoming-side event ───────────────────────────────
    // The backend listener saved the bytes under Downloads/PhantomChat/<...>
    // and emitted this event with the resolved sender label + verify result.
    // We surface the same `pendingUnboundPub` flow as text rows so the user
    // can bind an unfamiliar sender's pubkey from the file row too.
    listen<FileReceivedEvent>("file_received", e => {
      pushFileIncoming(e.payload);
      setScanned(s => s + 1);
      setDecrypted(d => d + 1);
      if (e.payload.from_label.startsWith("?") && e.payload.sender_pub_hex) {
        setPendingUnboundPub(e.payload.sender_pub_hex);
      }
    }).then(u => unlisteners.push(u));

    // ── Receipts: escalate matching outgoing row's delivery_state ────────
    // Only "sent" -> "delivered" -> "read" transitions are allowed. Late
    // "delivered" arrivals after "read" are dropped so the UI never
    // visually downgrades a row.
    listen<ReceiptEvent>("receipt", e => {
      const { msg_id, kind } = e.payload;
      const stateRank: Record<string, number> = {
        sent: 0,
        delivered: 1,
        read: 2,
      };
      setMessages(prev =>
        prev.map(m => {
          if (m.kind !== "outgoing") return m;
          if (!m.msg_id || m.msg_id !== msg_id) return m;
          const cur = m.delivery_state ?? "sent";
          if (stateRank[kind] <= stateRank[cur]) return m;
          return { ...m, delivery_state: kind };
        }),
      );
    }).then(u => unlisteners.push(u));

    // ── Typing indicators: stash expiry deadline + auto-clear ────────────
    // We don't track per-message ids — typing is a presence signal, not a
    // per-row state. The InputBar reads the active contact's entry; the
    // setTimeout fires a state update that drops the entry once the TTL
    // elapses so the pill disappears without manual cleanup.
    listen<TypingEvent>("typing", e => {
      const { from_label, ttl_secs } = e.payload;
      const expiry = Date.now() + ttl_secs * 1000;
      setTypingUntil(prev => {
        const next = new Map(prev);
        next.set(from_label, expiry);
        return next;
      });
      window.setTimeout(() => {
        setTypingUntil(prev => {
          // Only drop the entry if it still matches THIS expiry — a more
          // recent ping may have extended the deadline.
          const cur = prev.get(from_label);
          if (cur === undefined || cur > Date.now()) return prev;
          const next = new Map(prev);
          next.delete(from_label);
          return next;
        });
      }, ttl_secs * 1000 + 50);
    }).then(u => unlisteners.push(u));

    // ── Wave 8G: per-message pin/star state-change events ───────────────
    // Backend emits this whenever pin_message/unpin_message/star_message/
    // unstar_message succeeds. We patch the matching row in `messages`
    // by msg_id so the visual badge + tint flips without a reload.
    listen<MessageStateChangedEvent>("message_state_changed", e => {
      const { msg_id, pinned, starred } = e.payload;
      setMessages(prev =>
        prev.map(m =>
          m.msg_id === msg_id ? { ...m, pinned, starred } : m,
    // ── Reaction events ────────────────────────────────────────────────
    // Backend has already merged the `add`/`remove` action into the on-disk
    // history row's `reactions` array; we just patch in-memory state with
    // the post-merge list so the UI re-renders without a full reload.
    listen<ReactionUpdatedEvent>("reaction_updated", e => {
      const { target_msg_id, reactions } = e.payload;
      setMessages(prev =>
        prev.map(m =>
          m.msg_id === target_msg_id ? { ...m, reactions } : m,
        ),
      );
    }).then(u => unlisteners.push(u));

    // ── Wave 8G: per-conversation archive/pin/mute state-change events ──
    listen<ConversationStateChangedEvent>(
      "conversation_state_changed",
      e => {
        const { contact_label, state } = e.payload;
        setConversationState(prev => ({
          ...prev,
          [contact_label]: state,
        }));
        // If the user just archived the active conversation, drop the
        // selection so MessageStream doesn't keep showing rows from a
        // contact that is no longer in the live list.
        if (state.archived) {
          setActiveLabel(prev => (prev === contact_label ? null : prev));
        }
      },
    ).then(u => unlisteners.push(u));
    // ── Auto-purge: drop any row the backend just removed ──────────────
    // The 60s sweep on the Rust side mutates messages.json and emits this
    // with the dropped msg_ids. We mirror the change in React so the
    // disappearing rows vanish from the UI without waiting for a reload.
    listen<MessagesPurgedEvent>("messages_purged", e => {
      const dropped = new Set(e.payload.msg_ids);
      if (dropped.size === 0) return;
      setMessages(prev => prev.filter(m => !m.msg_id || !dropped.has(m.msg_id)));
    }).then(u => unlisteners.push(u));

    // ── Disappearing-TTL change → system message ───────────────────────
    // Fired both when WE change the TTL locally (via set_disappearing_ttl)
    // and when the PEER pushes a new TTL via DISA-1:. The system row
    // surfaces the change so both sides see a transcript trail.
    listen<DisappearingTtlChangedEvent>("disappearing_ttl_changed", e => {
      const { contact_label, ttl_secs } = e.payload;
      if (ttl_secs === null) {
        pushSystem(
          t("messages.disappearing.system_disabled", { label: contact_label }),
        );
      } else {
        pushSystem(
          t("messages.disappearing.system_enabled", {
            label: contact_label,
            secs: ttl_secs,
          }),
        );
      }
    }).then(u => unlisteners.push(u));

    return () => {
      unlisteners.forEach(u => u());
    };
  }, [pushMlsLog, refreshMlsStatus, myLabel]);
  }, [pushMlsLog, refreshMlsStatus, t]);

  // ── Send action ────────────────────────────────────────────────────────
  async function handleSend(body: string) {
    if (!activeLabel) {
      pushSystem(t("app.system.select_contact_first"));
      return;
    }
    try {
      // Look up any active disappearing-messages TTL for the active
      // contact so the optimistic outgoing row gets stamped with the
      // same `expires_at` deadline the peer will derive at receive time.
      // Single conditional, additive — falls back to undefined for
      // conversations with no TTL configured.
      const expiresAt =
        (await invoke<number | null>("outgoing_expires_at", {
          contactLabel: activeLabel,
        }).catch(() => null)) ?? undefined;
      const msgId = await invoke<string>("send_message", {
        contactLabel: activeLabel,
        body,
      });
      // Outgoing rows render with label "you" so the stream reads as a
      // proper conversation transcript when you re-open the app. The
      // returned msg_id stamps the row so subsequent `receipt` events
      // can escalate its delivery_state.
      pushOutgoing("you", body, msgId, { expires_at: expiresAt });
    } catch (e) {
      pushSystem(t("app.system.send_failed", { error: String(e) }));
    }
  }

  /// Reply-mode send wrapper. Routes through the backend `send_reply`
  /// command so the peer's incoming row carries the quote inline. The
  /// optimistic outgoing row is stamped with the same `reply_to` meta
  /// so it renders identically on both sides.
  async function handleSendReply(
    body: string,
    inReplyToMsgId: string,
    quotedPreview: string,
  ) {
    if (!activeLabel) {
      pushSystem(t("app.system.select_contact_first"));
      return;
    }
    try {
      const expiresAt =
        (await invoke<number | null>("outgoing_expires_at", {
          contactLabel: activeLabel,
        }).catch(() => null)) ?? undefined;
      const msgId = await invoke<string>("send_reply", {
        contactLabel: activeLabel,
        body,
        inReplyToMsgId,
        quotedPreview,
      });
      pushOutgoing("you", body, msgId, {
        reply_to: {
          in_reply_to_msg_id: inReplyToMsgId,
          quoted_preview: quotedPreview,
        },
        expires_at: expiresAt,
      });
      setReplyingTo(null);
    } catch (e) {
      pushSystem(t("app.system.send_failed", { error: String(e) }));
    }
  }

  /// React handler for the MessageStream toolbar / picker. Optimistically
  /// patches the local row's reactions array AND fires the backend
  /// `send_reaction` command so the peer's listener can mirror the
  /// change. The peer's `reaction_updated` event will then arrive and
  /// idempotently re-apply (the backend de-dupes identical adds).
  const handleReact = useCallback(
    async (
      msgId: string,
      emoji: string,
      action: "add" | "remove",
    ) => {
      if (!activeLabel) {
        pushSystem(t("app.system.select_contact_first"));
        return;
      }
      // Optimistic local mutation so the pill flips immediately. The
      // remote peer's mirroring add/remove will round-trip back via
      // `reaction_updated` and idempotently match this state.
      setMessages(prev =>
        prev.map(m => {
          if (m.msg_id !== msgId) return m;
          const cur = m.reactions ?? [];
          const exists = cur.some(
            r => r.sender_label === "you" && r.emoji === emoji,
          );
          let next = cur;
          if (action === "add" && !exists) {
            next = [...cur, { sender_label: "you", emoji }];
          } else if (action === "remove" && exists) {
            next = cur.filter(
              r => !(r.sender_label === "you" && r.emoji === emoji),
            );
          }
          return { ...m, reactions: next };
        }),
      );
      try {
        await invoke("send_reaction", {
          contactLabel: activeLabel,
          targetMsgId: msgId,
          emoji,
          action,
        });
      } catch (e) {
        pushSystem(t("app.system.send_failed", { error: String(e) }));
      }
    },
    [activeLabel, t],
  );

  /// File-send wrapper for both the paperclip button and the drag-drop
  /// overlay. Re-uses `pushSystem` for error surfacing so the failure
  /// (e.g. ">5 MiB cap" or "no contact selected") lands inline in the
  /// message stream rather than as a silent toast.
  async function handleSendFile(filePath: string): Promise<void> {
    if (!activeLabel) {
      pushSystem(t("app.system.select_contact_first"));
      return;
    }
    try {
      const result = await invoke<FileSendResult>("send_file", {
        contactLabel: activeLabel,
        filePath,
      });
      pushFileOutgoing("you", result);
    } catch (e) {
      pushSystem(t("app.system.file_send_failed", { error: String(e) }));
    }
  }

  /// "Open folder" affordance on file rows. Routes through the backend so
  /// we can centralize the Downloads/PhantomChat path resolution + lazy
  /// mkdir there, instead of having the React side ferry strings around.
  const handleOpenDownloadsFolder = useCallback(async () => {
    try {
      await invoke("open_downloads_folder");
    } catch (e) {
      pushSystem(t("app.system.open_folder_failed", { error: String(e) }));
    }
  }, []);

  /// Fired by MessageStream's IntersectionObserver the FIRST time an
  /// incoming row scrolls into view (and the window has focus). Backend
  /// builds + sends a sealed-sender `RCPT-1:` envelope with kind="read".
  /// Errors are swallowed quietly — the worst case is the sender sees
  /// ✓✓ instead of blue ✓✓, never a crash.
  const handleMarkRead = useCallback(
    (msgId: string, fromLabel: string) => {
      void invoke("mark_read", {
        msgId,
        contactLabel: fromLabel,
      }).catch(e => console.warn("mark_read failed:", e));
    },
    [],
  );

  /// Leading-edge throttled (1.5s) typing-ping invoked by InputBar on
  /// every keystroke. Best-effort fire-and-forget — a failed ping just
  /// means the peer's pill won't refresh; not worth surfacing.
  const handleTypingPing = useCallback((contactLabel: string) => {
    void invoke("typing_ping", { contactLabel }).catch(e =>
      console.warn("typing_ping failed:", e),
    );
  }, []);

  // ── Search panel keyboard shortcut (Ctrl/Cmd+F) ─────────────────────────
  // We toggle on the modifier+F combo, intercepting the browser's default
  // find-in-page (which is useless inside a Tauri webview anyway and would
  // double-render a native find UI on top of ours). Esc-to-close lives
  // inside SearchPanel itself so it only fires while the panel is open.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const isFindCombo =
        (e.ctrlKey || e.metaKey) && (e.key === "f" || e.key === "F");
      if (!isFindCombo) return;
      e.preventDefault();
      setShowSearch(prev => !prev);
      // Clear any prior highlight so reopening starts fresh — otherwise
      // a row from a previous search would still pulse on remount.
      if (showSearch) setSearchHighlightIdx(undefined);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showSearch]);

  // ── Drag-drop overlay state + handler ───────────────────────────────────
  // Tauri 2's webview emits `tauri://drag-enter` / `drag-over` / `drag-drop`
  // / `drag-leave` events. We surface a full-window neon overlay between
  // enter/leave so the user knows the drop target is live; on drop we route
  // each path through `handleSendFile`. Multi-file drops are accepted but
  // each file is sent as its own envelope (no batch wire format yet).
  const [isDragOver, setIsDragOver] = useState(false);
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    (async () => {
      try {
        unlisten = await getCurrentWebview().onDragDropEvent(event => {
          const payload = event.payload;
          if (payload.type === "enter" || payload.type === "over") {
            setIsDragOver(true);
          } else if (payload.type === "leave") {
            setIsDragOver(false);
          } else if (payload.type === "drop") {
            setIsDragOver(false);
            // Fire-and-forget — `handleSendFile` already surfaces failures.
            for (const p of payload.paths) {
              void handleSendFile(p);
            }
          }
        });
      } catch (e) {
        console.warn("onDragDropEvent register failed:", e);
      }
    })();
    return () => {
      unlisten?.();
    };
  }, [activeLabel]);

  async function handleAddContact(label: string, addr: string) {
    try {
      await invoke("add_contact", { label, address: addr });
      const cs = await invoke<Contact[]>("list_contacts");
      setContacts(cs);
      if (!activeLabel) setActiveLabel(label);
      setShowAddContact(false);
      pushSystem(t("app.system.added_contact", { label }));
    } catch (e) {
      pushSystem(t("app.system.add_contact_failed", { error: String(e) }));
    }
  }

  const handleBindToContact = useCallback(
    async (contactLabel: string) => {
      try {
        await invoke("bind_last_unbound_sender", { contactLabel });
        // Re-pull contacts so the unbound badge clears + signing_pub fills in.
        const cs = await invoke<Contact[]>("list_contacts");
        setContacts(cs);
        // Re-tag any in-memory ?<hex> rows whose pubkey matches the now-bound
        // contact's signing_pub. Future incoming messages will already arrive
        // with the contact label — this just patches up history.
        setMessages(prev =>
          prev.map(m => {
            if (
              m.kind === "incoming" &&
              m.label.startsWith("?") &&
              m.sender_pub_hex &&
              cs.some(
                c =>
                  c.label === contactLabel &&
                  c.signing_pub?.toLowerCase() ===
                    (m.sender_pub_hex ?? "").toLowerCase(),
              )
            ) {
              return { ...m, label: contactLabel };
            }
            return m;
          }),
        );
        setPendingUnboundPub(null);
        setShowBindModal(false);
        pushSystem(t("app.system.bound_sealed_sender", { label: contactLabel }));
      } catch (e) {
        pushSystem(t("app.system.bind_failed", { error: String(e) }));
      }
    },
    [],
  );

  async function handleGenerateIdentity() {
    try {
      const info = await invoke<{ address: string }>("generate_identity");
      setAddress(info.address);
      pushSystem(t("app.system.identity_generated"));
      // Re-trigger boot effect by reloading — easiest reliable path.
      window.location.reload();
    } catch (e) {
      setBootError(String(e));
    }
  }

  // ── Render ─────────────────────────────────────────────────────────────
  // Hold the screen blank while we're waiting on `is_onboarded` so we don't
  // briefly flash IdentityGate / main UI before the wizard claims the view.
  if (isOnboarded === null) {
    return <div className="h-screen w-screen bg-bg-deep" />;
  }

  if (isOnboarded === false) {
    return (
      <OnboardingWizard
        onDone={() => {
          // Clear the wizard, then let the boot effect re-fire by toggling
          // the gate. We force a reload because `start_listener` is gated
          // on `listenerStarted.current` which won't reset across renders.
          setIsOnboarded(true);
          window.location.reload();
        }}
      />
    );
  }

  if (address === null) {
    return (
      <IdentityGate onGenerate={handleGenerateIdentity} error={bootError} />
    );
  }

  return (
    <div className="flex flex-col h-screen bg-bg-deep text-neon-green font-mono select-none">
      {/* Update banner — passive, dismiss by opening Settings → About. */}
      {updateAvailable?.available && (
        <div className="px-4 py-1.5 text-xs text-cyber-cyan bg-cyber-cyan/10 border-b border-cyber-cyan/40 text-center font-display">
          {t("app.update_banner")}
        </div>
      )}
      {/* Header */}
      <header className="flex items-center justify-between px-4 py-2 border-b border-dim-green/40 bg-bg-panel/40 backdrop-blur-sm">
        <div className="flex items-center gap-3">
          <span className="text-neon-magenta font-bold text-lg pc-brand-glow-magenta font-display">P</span>
          <span className="text-neon-green font-bold tracking-widest pc-brand-glow font-display">
            PHANTOMCHAT
          </span>
          <span className="text-soft-grey">·</span>
          <span className="text-cyber-cyan text-xs font-display">{t("app.header.brand_subtitle")}</span>
        </div>
        <div className="text-xs text-soft-grey truncate max-w-[60%]" title={address}>
          {t("app.header.you_label")} <span className="text-neon-green">{shortAddr(address)}</span>
        </div>
      </header>

      {/* Tab switcher for left pane */}
      <div className="flex items-stretch border-b border-dim-green/40 bg-bg-panel text-xs uppercase tracking-widest">
        <button
          onClick={() => setLeftTab("contacts")}
          className={
            "px-4 py-1.5 transition-colors " +
            (leftTab === "contacts"
              ? "text-neon-green border-b-2 border-neon-green"
              : "text-soft-grey hover:text-cyber-cyan border-b-2 border-transparent")
          }
        >
          {t("app.tabs.contacts")}
        </button>
        <button
          onClick={() => setLeftTab("channels")}
          className={
            "px-4 py-1.5 transition-colors " +
            (leftTab === "channels"
              ? "text-neon-magenta border-b-2 border-neon-magenta"
              : "text-soft-grey hover:text-cyber-cyan border-b-2 border-transparent")
          }
        >
          {t("app.tabs.channels")}
        </button>
      </div>

      {/* Body: 3-pane */}
      <div className="flex flex-1 overflow-hidden">
        {/* `key` forces remount on tab toggle so the pc-tab-fade-in animation
            replays cleanly each switch. Cheaper than a manual exit-state
            machine and keeps both panes' internal state isolated. */}
        {leftTab === "contacts" ? (
          <div key="tab-contacts" className="flex pc-tab-fade-in">
            <ContactsPane
              contacts={contacts}
              activeLabel={activeLabel}
              onSelect={setActiveLabel}
              onAddClick={() => setShowAddContact(true)}
              hasUnboundSender={pendingUnboundPub !== null}
              onBindClick={() => setShowBindModal(true)}
              conversationState={conversationState}
            />
          </div>
        ) : (
          <div key="tab-channels" className="flex pc-tab-fade-in">
            <ChannelsPane
              log={mlsLog}
              pushLog={pushMlsLog}
              status={mlsStatus}
              refreshStatus={refreshMlsStatus}
            />
          </div>
        )}

        <main className="flex-1 flex flex-col relative">
          {/* Slide-down search panel — shown only when Ctrl/Cmd+F has
              toggled it on. Sits ABOVE the message stream so results
              don't push the input bar off-screen on short windows. */}
          {showSearch && (
            <SearchPanel
              contacts={contacts}
              onClose={() => setShowSearch(false)}
              onJumpTo={idx => {
                setSearchHighlightIdx(idx);
                setShowSearch(false);
              }}
            />
          )}
          <ConversationHeader
            activeLabel={activeLabel}
            onTtlChanged={() => {
              /* The backend's `set_disappearing_ttl` already emits the
                 `disappearing_ttl_changed` event, which our listener
                 turns into a system message — no extra work here. */
            }}
          />
          <MessageStream
            messages={messages}
            activeLabel={activeLabel}
            onBindClick={() => setShowBindModal(true)}
            onOpenFolder={handleOpenDownloadsFolder}
            highlightedIdx={searchHighlightIdx}
            onMarkRead={handleMarkRead}
            onSwitchConversation={setActiveLabel}
            knownMentionLabels={[
              myLabel,
              ...(mlsStatus?.members.map(m => m.label) ?? []),
              ...contacts.map(c => c.label),
            ].filter(Boolean)}
            onReplyTo={(msgId, preview) => {
              setReplyingTo({
                msg_id: msgId,
                // Preview is capped at the same ~80 chars the backend
                // uses for the wire metadata so the on-screen quote
                // matches the eventual envelope contents.
                preview:
                  preview.length > 80
                    ? preview.slice(0, 80) + "\u{2026}"
                    : preview,
              });
            }}
            onReact={handleReact}
          />
          <InputBar
            activeLabel={activeLabel}
            onSend={handleSend}
            onSendFile={handleSendFile}
            onTypingPing={handleTypingPing}
            typingFromLabel={(() => {
              if (!activeLabel) return null;
              const exp = typingUntil.get(activeLabel);
              if (exp === undefined) return null;
              return exp > Date.now() ? activeLabel : null;
            })()}
            // Mention auto-complete is only meaningful inside an MLS
            // group (1:1 chats have a single peer — no need to disambig-
            // uate). Pass the directory only when we're on the channels
            // tab AND a group is active; otherwise the popover stays
            // suppressed inside InputBar.
            mlsDirectory={
              leftTab === "channels" && mlsStatus?.in_group
                ? mlsStatus.members
                : undefined
            }
            replyingTo={replyingTo}
            onCancelReply={() => setReplyingTo(null)}
            onSendReply={handleSendReply}
          />
          {/* Drag-drop overlay — sits absolute over the chat pane so the
              dimmed message stream shows through. Pointer-events-none so
              the underlying drag-drop event still fires on the webview. */}
          {isDragOver && (
            <div className="pointer-events-none absolute inset-0 z-30 flex items-center justify-center bg-bg-deep/70 border-2 border-dashed border-cyber-cyan rounded-md">
              <div className="text-cyber-cyan text-sm uppercase tracking-widest font-display">
                {t("app.drop_overlay", { label: activeLabel ?? t("app.drop_overlay_no_contact") })}
              </div>
            </div>
          )}
        </main>
      </div>

      <StatusFooter
        scanned={scanned}
        decrypted={decrypted}
        relay={DEFAULT_RELAY}
        connection={connection}
        onOpenSettings={() => setShowSettings(true)}
      />

      {showAddContact && (
        <AddContactModal
          onClose={() => setShowAddContact(false)}
          onSubmit={handleAddContact}
        />
      )}

      {showBindModal && (
        <BindContactModal
          pubHex={pendingUnboundPub}
          contacts={contacts}
          onClose={() => setShowBindModal(false)}
          onBind={handleBindToContact}
        />
      )}

      {showSettings && (
        <SettingsPanel onClose={() => setShowSettings(false)} />
      )}
    </div>
  );
}

function shortAddr(addr: string): string {
  const stripped = addr.replace(/^phantomx?:/, "");
  if (stripped.length < 16) return addr;
  return `${addr.startsWith("phantomx:") ? "phantomx:" : "phantom:"}${stripped.slice(0, 8)}…${stripped.slice(-6)}`;
}
