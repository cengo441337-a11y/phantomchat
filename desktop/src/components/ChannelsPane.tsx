import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import type { MlsLogLine, MlsStatus } from "../types";

async function copyToClipboard(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Fallback: silently no-op. The base64 stays visible for manual copy.
  }
}

interface CopyBlockProps {
  label: string;
  value: string;
}

function CopyBlock({ label, value }: CopyBlockProps) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between">
        <span className="text-[10px] uppercase tracking-widest text-soft-grey">
          {label}
        </span>
        <button
          onClick={async () => {
            await copyToClipboard(value);
            setCopied(true);
            setTimeout(() => setCopied(false), 1200);
          }}
          className="text-[10px] uppercase tracking-wider text-cyber-cyan hover:text-neon-green"
        >
          {copied ? t("channels_pane.copy.copied") : t("channels_pane.copy.copy")}
        </button>
      </div>
      <pre className="bg-bg-deep border border-dim-green/40 rounded-md px-2 py-1.5 text-[10px] text-neon-green/80 max-h-24 overflow-y-auto whitespace-pre-wrap break-all">
        {value}
      </pre>
    </div>
  );
}

interface ChannelsPaneProps {
  /// Lifted log state — owned by App.tsx so events keep flowing into it
  /// even when this pane is hidden (tab switched to contacts). On remount
  /// the user sees the full history.
  log: MlsLogLine[];
  pushLog: (kind: MlsLogLine["kind"], body: string) => void;
  /// Latest `mls_status` snapshot. App.tsx refreshes it after every
  /// auto-event so the directory + member_count stay current here even
  /// when ChannelsPane is unmounted.
  status: MlsStatus | null;
  refreshStatus: () => Promise<void>;
}

export default function ChannelsPane({
  log,
  pushLog,
  status,
  refreshStatus,
}: ChannelsPaneProps) {
  const { t } = useTranslation();
  // Init / create
  const [identityLabel, setIdentityLabel] = useState("");

  // Share / invite — local UI state for the KP-display + add-member form.
  // The 4-field add-member form replaces the legacy single-textarea path:
  // the inviter must now also supply the peer's PhantomAddress + signing
  // pubkey so the backend can ship the welcome via sealed-sender.
  const [myKp, setMyKp] = useState<string | null>(null);
  const [peerKp, setPeerKp] = useState("");
  const [peerLabel, setPeerLabel] = useState("");
  const [peerAddress, setPeerAddress] = useState("");
  const [peerSigningPub, setPeerSigningPub] = useState("");

  // Send (auto-published — no separate decrypt input anymore)
  const [plaintext, setPlaintext] = useState("");

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  async function handleInit() {
    if (!identityLabel.trim()) return;
    try {
      await invoke("mls_init", { identityLabel: identityLabel.trim() });
      pushLog("system", t("channels_pane.init.initialized", { label: identityLabel.trim() }));
      await refreshStatus();
    } catch (e) {
      pushLog("system", t("channels_pane.init.init_failed", { error: String(e) }));
    }
  }

  async function handleCreateGroup() {
    try {
      await invoke("mls_create_group");
      pushLog("system", t("channels_pane.create.created"));
      await refreshStatus();
    } catch (e) {
      pushLog("system", t("channels_pane.create.create_failed", { error: String(e) }));
    }
  }

  async function handleShowKp() {
    try {
      const b64 = await invoke<string>("mls_publish_key_package");
      setMyKp(b64);
      pushLog("system", t("channels_pane.share_kp.kp_generated", { chars: b64.length }));
    } catch (e) {
      pushLog("system", t("channels_pane.share_kp.publish_failed", { error: String(e) }));
    }
  }

  async function handleAddMember() {
    if (!peerKp.trim() || !peerLabel.trim() || !peerAddress.trim() || !peerSigningPub.trim()) {
      return;
    }
    try {
      await invoke("mls_add_member", {
        keyPackageB64: peerKp.trim(),
        memberLabel: peerLabel.trim(),
        memberAddress: peerAddress.trim(),
        memberSigningPubHex: peerSigningPub.trim(),
      });
      pushLog(
        "system",
        t("channels_pane.invite.invited", { label: peerLabel.trim() }),
      );
      setPeerKp("");
      setPeerLabel("");
      setPeerAddress("");
      setPeerSigningPub("");
      await refreshStatus();
    } catch (e) {
      pushLog("system", t("channels_pane.invite.add_failed", { error: String(e) }));
    }
  }

  async function handleSendToGroup() {
    if (!plaintext.trim()) return;
    const body = plaintext;
    try {
      // mls_encrypt now returns () — fan-out happens in the backend.
      await invoke("mls_encrypt", { plaintext: body });
      pushLog("outgoing", body);
      setPlaintext("");
      await refreshStatus();
    } catch (e) {
      pushLog("system", t("channels_pane.send.send_failed", { error: String(e) }));
    }
  }

  const initialized = status?.initialized ?? false;
  const inGroup = status?.in_group ?? false;
  const directorySize = status?.members.length ?? 0;
  const addMemberReady =
    !!peerKp.trim() && !!peerLabel.trim() && !!peerAddress.trim() && !!peerSigningPub.trim();

  return (
    <aside className="w-[380px] shrink-0 flex flex-col border-r border-dim-green/40 bg-bg-panel/85 backdrop-blur-[1px] overflow-y-auto pc-pane pc-pane-magenta">
      <div className="flex items-center justify-between px-3 py-2 border-b border-dim-green/40 sticky top-0 bg-bg-panel/90 backdrop-blur-sm z-10">
        <span className="text-neon-green text-xs uppercase tracking-widest font-display">
          {t("channels_pane.header")}
        </span>
        <button
          onClick={() => void refreshStatus()}
          className="text-cyber-cyan text-xs hover:text-neon-green"
          title={t("channels_pane.refresh_title")}
        >
          ⟳
        </button>
      </div>

      <div className="p-3 space-y-4 text-xs">
        {/* Status block */}
        <section className="panel-border p-2 space-y-1">
          <div className="text-[10px] uppercase tracking-widest text-cyber-cyan font-display">
            {t("channels_pane.status.title")}
          </div>
          <div className="flex justify-between">
            <span className="text-soft-grey">{t("channels_pane.status.initialized")}</span>
            <span className={initialized ? "text-neon-green" : "text-neon-magenta"}>
              {initialized ? t("channels_pane.status.yes") : t("channels_pane.status.no")}
            </span>
          </div>
          <div className="flex justify-between">
            <span className="text-soft-grey">{t("channels_pane.status.in_group")}</span>
            <span className={inGroup ? "text-neon-green" : "text-neon-magenta"}>
              {inGroup ? t("channels_pane.status.yes") : t("channels_pane.status.no")}
            </span>
          </div>
          <div className="flex justify-between">
            <span className="text-soft-grey">{t("channels_pane.status.members_mls")}</span>
            <span className="text-neon-green">{status?.member_count ?? 0}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-soft-grey">{t("channels_pane.status.directory")}</span>
            <span className="text-neon-green">{directorySize}</span>
          </div>
          {status?.identity_label && (
            <div className="flex justify-between">
              <span className="text-soft-grey">{t("channels_pane.status.identity")}</span>
              <span className="text-cyber-cyan truncate ml-2">
                {status.identity_label}
              </span>
            </div>
          )}
          <div
            className="text-[10px] text-soft-grey/70 italic pt-1 border-t border-dim-green/30 mt-1"
          >
            {t("channels_pane.status.ram_only_hint")}
          </div>
        </section>

        {/* Init */}
        {!initialized && (
          <section className="panel-border p-2 space-y-2">
            <div className="text-[10px] uppercase tracking-widest text-cyber-cyan font-display">
              {t("channels_pane.init.title")}
            </div>
            <input
              type="text"
              value={identityLabel}
              onChange={e => setIdentityLabel(e.target.value)}
              placeholder={t("channels_pane.init.label_placeholder")}
              className="neon-input w-full text-xs"
            />
            <button
              onClick={() => void handleInit()}
              disabled={!identityLabel.trim()}
              className="neon-button w-full text-xs disabled:opacity-40"
            >
              {t("channels_pane.init.init_button")}
            </button>
          </section>
        )}

        {/* Create group (or just wait — incoming Welcome auto-joins) */}
        {initialized && !inGroup && (
          <section className="panel-border p-2 space-y-3">
            <div className="text-[10px] uppercase tracking-widest text-cyber-cyan font-display">
              {t("channels_pane.create.title")}
            </div>
            <button
              onClick={() => void handleCreateGroup()}
              className="neon-button w-full text-xs"
            >
              {t("channels_pane.create.create_button")}
            </button>
            <div className="border-t border-dim-green/30 pt-2 text-[10px] text-soft-grey italic">
              {t("channels_pane.create.wait_hint")}
            </div>
          </section>
        )}

        {/* Share KP — always available once initialized */}
        {initialized && (
          <section className="panel-border p-2 space-y-2">
            <div className="text-[10px] uppercase tracking-widest text-cyber-cyan font-display">
              {t("channels_pane.share_kp.title")}
            </div>
            <button
              onClick={() => void handleShowKp()}
              className="neon-button w-full text-xs"
            >
              {myKp ? t("channels_pane.share_kp.regenerate_button") : t("channels_pane.share_kp.show_button")}
            </button>
            {myKp && <CopyBlock label={t("channels_pane.share_kp.kp_label")} value={myKp} />}
          </section>
        )}

        {/* Invite — 4-field form. Welcome auto-flies; nothing to display. */}
        {initialized && inGroup && (
          <section className="panel-border p-2 space-y-2">
            <div className="text-[10px] uppercase tracking-widest text-cyber-cyan font-display">
              {t("channels_pane.invite.title")}
            </div>
            <textarea
              value={peerKp}
              onChange={e => setPeerKp(e.target.value)}
              placeholder={t("channels_pane.invite.kp_placeholder")}
              rows={3}
              className="neon-input w-full text-[10px] font-mono resize-y"
            />
            <input
              type="text"
              value={peerLabel}
              onChange={e => setPeerLabel(e.target.value)}
              placeholder={t("channels_pane.invite.label_placeholder")}
              className="neon-input w-full text-xs"
            />
            <input
              type="text"
              value={peerAddress}
              onChange={e => setPeerAddress(e.target.value)}
              placeholder={t("channels_pane.invite.address_placeholder")}
              className="neon-input w-full text-[10px] font-mono"
            />
            <input
              type="text"
              value={peerSigningPub}
              onChange={e => setPeerSigningPub(e.target.value)}
              placeholder={t("channels_pane.invite.signing_placeholder")}
              className="neon-input w-full text-[10px] font-mono"
            />
            <button
              onClick={() => void handleAddMember()}
              disabled={!addMemberReady}
              className="neon-button w-full text-xs disabled:opacity-40"
            >
              {t("channels_pane.invite.add_button")}
            </button>
            <div className="text-[10px] text-soft-grey italic">
              {t("channels_pane.invite.auto_ship_hint")}
            </div>
          </section>
        )}

        {/* Directory snapshot */}
        {initialized && inGroup && directorySize > 0 && (
          <section className="panel-border p-2 space-y-1">
            <div className="text-[10px] uppercase tracking-widest text-cyber-cyan font-display">
              {t("channels_pane.directory.title")}
            </div>
            {status?.members.map(m => (
              <div
                key={m.signing_pub_hex}
                className="text-[10px] font-mono flex justify-between"
              >
                <span className="text-neon-green">{m.label}</span>
                <span
                  className="text-soft-grey truncate ml-2"
                  title={m.address}
                >
                  {m.address.slice(0, 24)}…
                </span>
              </div>
            ))}
          </section>
        )}

        {/* Send-to-group (auto-publishes; no separate decrypt input) */}
        {initialized && inGroup && (
          <section className="panel-border p-2 space-y-3">
            <div className="text-[10px] uppercase tracking-widest text-cyber-cyan font-display">
              {t("channels_pane.send.title")}
            </div>
            <input
              type="text"
              value={plaintext}
              onChange={e => setPlaintext(e.target.value)}
              onKeyDown={e => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void handleSendToGroup();
                }
              }}
              placeholder={t("channels_pane.send.placeholder")}
              className="neon-input w-full text-xs"
            />
            <button
              onClick={() => void handleSendToGroup()}
              disabled={!plaintext.trim()}
              className="neon-button w-full text-xs disabled:opacity-40"
            >
              {t("channels_pane.send.send_button")}
            </button>
            <div className="text-[10px] text-soft-grey italic">
              {directorySize === 1
                ? t("channels_pane.send.fanout_hint_one", { count: directorySize })
                : t("channels_pane.send.fanout_hint_other", { count: directorySize })}
            </div>
          </section>
        )}

        {/* Group message log */}
        <section className="panel-border p-2 space-y-1">
          <div className="text-[10px] uppercase tracking-widest text-cyber-cyan font-display">
            {t("channels_pane.log.title")}
          </div>
          {log.length === 0 ? (
            <div className="text-soft-grey italic text-[10px]">
              {t("channels_pane.log.empty")}
            </div>
          ) : (
            <div className="space-y-1 max-h-64 overflow-y-auto">
              {log.map((l, i) => {
                const arrow =
                  l.kind === "incoming" ? "◀" : l.kind === "outgoing" ? "▶" : "·";
                const arrowColor =
                  l.kind === "incoming"
                    ? "text-cyber-cyan"
                    : l.kind === "outgoing"
                    ? "text-neon-green"
                    : "text-soft-grey";
                return (
                  <div key={i} className="flex items-start gap-2 text-[11px] font-mono">
                    <span className="text-soft-grey w-[56px] shrink-0">
                      {l.ts}
                    </span>
                    <span className={`${arrowColor} w-3 shrink-0`}>{arrow}</span>
                    <span
                      className={
                        l.kind === "system"
                          ? "text-soft-grey italic break-words"
                          : "text-neon-green/90 break-words"
                      }
                    >
                      {l.body}
                    </span>
                  </div>
                );
              })}
            </div>
          )}
        </section>
      </div>
    </aside>
  );
}
