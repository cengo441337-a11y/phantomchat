import { invoke } from "@tauri-apps/api/core";
import { appDataDir, join } from "@tauri-apps/api/path";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useEffect, useMemo, useState } from "react";

/**
 * Argos Wallet panel — desktop sibling of mobile/lib/screens/wallet_screen.dart.
 *
 * State machine — same five stages as mobile so the UX is identical:
 *   loading -> none | locked | main
 *   none -> createPin -> backupReveal -> main
 *   none -> restore -> main
 *   locked -> main
 *
 * The cached `Arc<ArgosWallet>` lives Rust-side (src-tauri/src/wallet_cmds.rs),
 * so the React layer holds only the public pubkey + the in-flight network
 * call state. No mnemonic or PIN persists in the TypeScript heap beyond
 * the create-or-restore flow.
 */
type Stage =
  | "loading"
  | "none"
  | "createPin"
  | "backupReveal"
  | "restore"
  | "locked"
  | "main";

type Net = "mainnet-beta" | "devnet";

interface WalletInfo {
  pubkey_b58: string;
  mnemonic: string;
  network: string;
}

interface SwapPreview {
  amount_in: number;
  amount_out_min: number;
  amount_out_expected: number;
  platform_fee_out: number;
  route_label: string;
  slippage_bps: number;
  output_mint_b58: string;
}

const KNOWN_TOKENS = [
  {
    symbol: "USDC",
    name: "USD Coin",
    mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
    decimals: 6,
  },
  {
    symbol: "USDT",
    name: "Tether",
    mint: "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
    decimals: 6,
  },
];

const WSOL_MINT = "So11111111111111111111111111111111111111112";

function shortAddr(s: string): string {
  if (s.length <= 12) return s;
  return s.slice(0, 5) + "…" + s.slice(-5);
}

function solHuman(lamports: number): string {
  const whole = Math.floor(lamports / 1_000_000_000);
  const frac = lamports % 1_000_000_000;
  return `${whole}.${String(frac).padStart(9, "0").slice(0, 4)}`;
}

function tokenHuman(raw: number, decimals: number): string {
  if (decimals === 0) return String(raw);
  const pow = Math.pow(10, decimals);
  const whole = Math.floor(raw / pow);
  const frac = raw % pow;
  return `${whole}.${String(frac).padStart(decimals, "0").slice(0, Math.min(decimals, 4))}`;
}

export function WalletPanel({ appVersion }: { appVersion: string }) {
  const [stage, setStage] = useState<Stage>("loading");
  const [pubkey, setPubkey] = useState<string | null>(null);
  const [network, setNetwork] = useState<Net>("mainnet-beta");
  const [storagePath, setStoragePath] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [justRevealedMnemonic, setJustRevealedMnemonic] = useState<string | null>(null);

  // Main-view state
  const [solLamports, setSolLamports] = useState<number>(0);
  const [tokenBalances, setTokenBalances] = useState<Record<string, number>>({});
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    (async () => {
      try {
        const dir = await appDataDir();
        const path = await join(dir, "argos_wallet.enc.json");
        setStoragePath(path);
        // Probe Rust-side cache to see if the wallet is already unlocked
        // (e.g. user just came back from another tab).
        const cachedPubkey = await invoke<string | null>("argos_wallet_pubkey");
        if (cachedPubkey) {
          setPubkey(cachedPubkey);
          const net = (await invoke<string | null>("argos_wallet_network")) ?? "mainnet-beta";
          setNetwork(net as Net);
          setStage("main");
          void refresh();
          return;
        }
        // Otherwise look on disk: if the file exists, go to locked.
        // We do not query the filesystem from the TS side; if unlock-
        // succeeds we are good, if not the user picks from "none".
        // Cheap heuristic: try a no-op unlock with an empty PIN — Argos
        // returns WrongPin or "file not found" and we branch on the message.
        try {
          await invoke<string>("argos_unlock_wallet", { pin: "", storagePath: path });
          // Empty PIN unlocked (impossible) — bail to none.
          setStage("none");
        } catch (e) {
          const msg = String(e);
          if (msg.includes("No such file") || msg.includes("not found") || msg.includes("kein") || msg.includes("Datei")) {
            setStage("none");
          } else {
            setStage("locked");
          }
        }
      } catch (e) {
        setError(String(e));
        setStage("none");
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function refresh() {
    if (!pubkey && stage !== "main") return;
    setRefreshing(true);
    try {
      const sol = await invoke<number>("argos_balance_sol");
      setSolLamports(sol);
      const tokens: Record<string, number> = {};
      for (const t of KNOWN_TOKENS) {
        try {
          tokens[t.mint] = await invoke<number>("argos_balance_token", {
            mintB58: t.mint,
          });
        } catch {
          tokens[t.mint] = 0;
        }
      }
      setTokenBalances(tokens);
    } catch (e) {
      setError(String(e));
    } finally {
      setRefreshing(false);
    }
  }

  async function onCreate(pin: string) {
    setError(null);
    try {
      const info = await invoke<WalletInfo>("argos_create_wallet", {
        network,
        pin,
        storagePath,
      });
      setPubkey(info.pubkey_b58);
      setJustRevealedMnemonic(info.mnemonic);
      setStage("backupReveal");
    } catch (e) {
      setError(String(e));
    }
  }

  async function onRestore(mnemonic: string, pin: string) {
    setError(null);
    try {
      const info = await invoke<WalletInfo>("argos_restore_wallet", {
        mnemonic,
        network,
        pin,
        storagePath,
      });
      setPubkey(info.pubkey_b58);
      setStage("main");
      void refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function onUnlock(pin: string) {
    setError(null);
    try {
      const pk = await invoke<string>("argos_unlock_wallet", {
        pin,
        storagePath,
      });
      setPubkey(pk);
      const net = (await invoke<string | null>("argos_wallet_network")) ?? "mainnet-beta";
      setNetwork(net as Net);
      setStage("main");
      void refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function onLock() {
    await invoke("argos_lock_wallet");
    setPubkey(null);
    setStage("locked");
  }

  return (
    <div className="flex flex-col h-full text-neon-green font-mono">
      <header className="flex items-center justify-between px-6 py-3 border-b border-dim-green/40">
        <div className="flex items-center gap-3">
          <span className="text-neon-magenta font-bold pc-brand-glow-magenta font-display">A</span>
          <span className="font-bold tracking-widest pc-brand-glow font-display text-lg">
            ARGOS WALLET
          </span>
          {appVersion && (
            <span className="text-soft-grey text-[10px]">v{appVersion}</span>
          )}
        </div>
        {stage === "main" && (
          <button
            onClick={onLock}
            className="text-soft-grey hover:text-cyber-cyan text-xs uppercase tracking-widest"
          >
            ⌧ Lock
          </button>
        )}
      </header>

      <div className="flex-1 overflow-y-auto p-6">
        {stage === "loading" && (
          <div className="flex justify-center pt-20 text-cyber-cyan">⟳ lade…</div>
        )}
        {stage === "none" && (
          <OnboardingChoice
            network={network}
            setNetwork={setNetwork}
            onCreate={() => setStage("createPin")}
            onRestore={() => setStage("restore")}
          />
        )}
        {stage === "createPin" && (
          <CreatePinPanel
            onCreate={onCreate}
            onCancel={() => setStage("none")}
            err={error}
            network={network}
          />
        )}
        {stage === "backupReveal" && justRevealedMnemonic && (
          <BackupRevealPanel
            mnemonic={justRevealedMnemonic}
            onConfirmed={() => {
              setJustRevealedMnemonic(null);
              setStage("main");
              void refresh();
            }}
          />
        )}
        {stage === "restore" && (
          <RestorePanel
            network={network}
            setNetwork={setNetwork}
            onRestore={onRestore}
            onCancel={() => setStage("none")}
            err={error}
          />
        )}
        {stage === "locked" && (
          <UnlockPanel onUnlock={onUnlock} err={error} />
        )}
        {stage === "main" && pubkey && (
          <MainPanel
            pubkey={pubkey}
            network={network}
            solLamports={solLamports}
            tokenBalances={tokenBalances}
            refreshing={refreshing}
            onRefresh={refresh}
          />
        )}
      </div>
    </div>
  );
}

// ── Sub-panels ──────────────────────────────────────────────────────────

function NetworkToggle({
  network,
  setNetwork,
}: {
  network: Net;
  setNetwork: (n: Net) => void;
}) {
  return (
    <div className="flex gap-2 mb-4">
      {(["mainnet-beta", "devnet"] as Net[]).map((n) => (
        <button
          key={n}
          onClick={() => setNetwork(n)}
          className={
            "flex-1 py-2 border text-xs uppercase tracking-widest " +
            (network === n
              ? "border-cyber-cyan bg-cyber-cyan/10 text-cyber-cyan"
              : "border-dim-green/40 text-soft-grey hover:text-neon-green")
          }
        >
          {n === "mainnet-beta" ? "MAINNET" : "DEVNET"}
        </button>
      ))}
    </div>
  );
}

function OnboardingChoice({
  network,
  setNetwork,
  onCreate,
  onRestore,
}: {
  network: Net;
  setNetwork: (n: Net) => void;
  onCreate: () => void;
  onRestore: () => void;
}) {
  return (
    <div className="max-w-md mx-auto pt-12">
      <div className="text-center mb-10">
        <div className="inline-block w-20 h-20 border-2 border-cyber-cyan flex items-center justify-center mb-4">
          <span className="text-cyber-cyan text-5xl font-bold font-display">A</span>
        </div>
        <h2 className="font-display text-xl tracking-widest mb-2">ARGOS WALLET</h2>
        <p className="text-soft-grey text-xs">
          Non-custodial Solana · BIP39 + Argon2id · Auto-Swap-on-Send
        </p>
      </div>
      <NetworkToggle network={network} setNetwork={setNetwork} />
      <button
        onClick={onCreate}
        className="w-full py-4 border-2 border-cyber-cyan text-cyber-cyan uppercase tracking-widest font-display hover:bg-cyber-cyan/10"
      >
        Neue Wallet erstellen
      </button>
      <button
        onClick={onRestore}
        className="w-full py-3 mt-3 text-soft-grey hover:text-cyber-cyan text-xs uppercase tracking-widest"
      >
        Bestehende Wallet wiederherstellen
      </button>
      <p className="text-soft-grey text-[10px] text-center mt-12">
        Schlüssel verlassen NIE dieses Gerät.<br />
        Cloud-Sync · Telemetrie · Custodian = 0
      </p>
    </div>
  );
}

function CreatePinPanel({
  onCreate,
  onCancel,
  err,
  network,
}: {
  onCreate: (pin: string) => void;
  onCancel: () => void;
  err: string | null;
  network: Net;
}) {
  const [p1, setP1] = useState("");
  const [p2, setP2] = useState("");
  const [busy, setBusy] = useState(false);
  function go() {
    if (p1.length < 6) return;
    if (p1 !== p2) return;
    setBusy(true);
    onCreate(p1);
  }
  return (
    <div className="max-w-md mx-auto pt-12">
      <h2 className="font-display tracking-widest text-lg mb-2 text-cyber-cyan">PIN FESTLEGEN</h2>
      <p className="text-soft-grey text-xs mb-6">
        Verschlüsselt die Recovery mit Argon2id auf {network}. 10 falsche Versuche → Wipe.
      </p>
      <input
        type="password"
        value={p1}
        onChange={(e) => setP1(e.target.value.replace(/\D/g, ""))}
        placeholder="NEUE PIN (≥6)"
        className="w-full px-3 py-2 mb-2 bg-bg-deep border border-dim-green/40 text-cyber-cyan tracking-widest"
      />
      <input
        type="password"
        value={p2}
        onChange={(e) => setP2(e.target.value.replace(/\D/g, ""))}
        placeholder="PIN BESTÄTIGEN"
        className="w-full px-3 py-2 bg-bg-deep border border-dim-green/40 text-cyber-cyan tracking-widest"
      />
      {err && <p className="text-neon-magenta text-xs mt-2">{err}</p>}
      <button
        disabled={busy || p1.length < 6 || p1 !== p2}
        onClick={go}
        className="w-full py-3 mt-6 border-2 border-cyber-cyan text-cyber-cyan disabled:opacity-30 uppercase tracking-widest font-display"
      >
        {busy ? "Erstellen…" : "Wallet erstellen"}
      </button>
      <button
        onClick={onCancel}
        className="w-full py-2 mt-3 text-soft-grey hover:text-cyber-cyan text-xs uppercase tracking-widest"
      >
        Abbrechen
      </button>
    </div>
  );
}

function BackupRevealPanel({
  mnemonic,
  onConfirmed,
}: {
  mnemonic: string;
  onConfirmed: () => void;
}) {
  const [revealed, setRevealed] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const words = mnemonic.trim().split(/\s+/);
  return (
    <div className="max-w-2xl mx-auto pt-8">
      <h2 className="font-display tracking-widest text-lg mb-2 text-neon-magenta">RECOVERY-PHRASE</h2>
      <p className="text-soft-grey text-xs mb-6">
        Diese 24 Wörter sind der EINZIGE Weg, deine Wallet wiederherzustellen.
        Schreib sie auf Papier. NIE als Screenshot / Cloud-Backup.
      </p>
      {revealed ? (
        <div className="grid grid-cols-3 gap-2 p-4 border border-cyber-cyan bg-bg-deep">
          {words.map((w, i) => (
            <div key={i} className="text-xs px-2 py-1 border border-cyber-cyan/30">
              <span className="text-soft-grey mr-1">{String(i + 1).padStart(2, "0")}</span>
              <span className="text-cyber-cyan font-bold">{w}</span>
            </div>
          ))}
        </div>
      ) : (
        <button
          onClick={() => setRevealed(true)}
          className="w-full py-12 border border-dim-green/40 bg-bg-deep text-soft-grey hover:text-neon-magenta uppercase tracking-widest"
        >
          ◊ Tap zum Aufdecken — schau dich um
        </button>
      )}
      {revealed && (
        <>
          <label className="flex items-center gap-2 mt-4 text-xs cursor-pointer">
            <input
              type="checkbox"
              checked={confirmed}
              onChange={(e) => setConfirmed(e.target.checked)}
            />
            <span>Ich habe alle 24 Wörter aufgeschrieben.</span>
          </label>
          <button
            disabled={!confirmed}
            onClick={onConfirmed}
            className="w-full py-3 mt-4 border-2 border-cyber-cyan text-cyber-cyan disabled:opacity-30 uppercase tracking-widest font-display"
          >
            Weiter zur Wallet
          </button>
        </>
      )}
    </div>
  );
}

function RestorePanel({
  network,
  setNetwork,
  onRestore,
  onCancel,
  err,
}: {
  network: Net;
  setNetwork: (n: Net) => void;
  onRestore: (mnemonic: string, pin: string) => void;
  onCancel: () => void;
  err: string | null;
}) {
  const [mn, setMn] = useState("");
  const [p1, setP1] = useState("");
  const [p2, setP2] = useState("");
  return (
    <div className="max-w-md mx-auto pt-8">
      <h2 className="font-display tracking-widest text-lg mb-4 text-cyber-cyan">
        WALLET WIEDERHERSTELLEN
      </h2>
      <NetworkToggle network={network} setNetwork={setNetwork} />
      <textarea
        rows={4}
        value={mn}
        onChange={(e) => setMn(e.target.value)}
        placeholder="12 oder 24 Wörter, durch Leerzeichen getrennt"
        className="w-full px-3 py-2 mb-2 bg-bg-deep border border-dim-green/40 text-cyber-cyan text-sm"
      />
      <input
        type="password"
        value={p1}
        onChange={(e) => setP1(e.target.value.replace(/\D/g, ""))}
        placeholder="NEUE PIN (≥6)"
        className="w-full px-3 py-2 mb-2 bg-bg-deep border border-dim-green/40 text-cyber-cyan tracking-widest"
      />
      <input
        type="password"
        value={p2}
        onChange={(e) => setP2(e.target.value.replace(/\D/g, ""))}
        placeholder="PIN BESTÄTIGEN"
        className="w-full px-3 py-2 bg-bg-deep border border-dim-green/40 text-cyber-cyan tracking-widest"
      />
      {err && <p className="text-neon-magenta text-xs mt-2">{err}</p>}
      <button
        disabled={mn.trim().split(/\s+/).length < 12 || p1.length < 6 || p1 !== p2}
        onClick={() => onRestore(mn, p1)}
        className="w-full py-3 mt-6 border-2 border-cyber-cyan text-cyber-cyan disabled:opacity-30 uppercase tracking-widest font-display"
      >
        Wiederherstellen
      </button>
      <button
        onClick={onCancel}
        className="w-full py-2 mt-3 text-soft-grey hover:text-cyber-cyan text-xs uppercase tracking-widest"
      >
        Abbrechen
      </button>
    </div>
  );
}

function UnlockPanel({
  onUnlock,
  err,
}: {
  onUnlock: (pin: string) => void;
  err: string | null;
}) {
  const [pin, setPin] = useState("");
  return (
    <div className="max-w-sm mx-auto pt-20 text-center">
      <div className="text-5xl mb-4">⌧</div>
      <h2 className="font-display tracking-widest text-lg mb-6 text-cyber-cyan">
        WALLET ENTSPERREN
      </h2>
      <input
        type="password"
        value={pin}
        onChange={(e) => setPin(e.target.value.replace(/\D/g, ""))}
        onKeyDown={(e) => e.key === "Enter" && onUnlock(pin)}
        placeholder="PIN"
        className="w-full px-3 py-3 bg-bg-deep border border-dim-green/40 text-cyber-cyan tracking-[0.5em] text-center text-lg"
      />
      {err && <p className="text-neon-magenta text-xs mt-2">{err}</p>}
      <button
        onClick={() => onUnlock(pin)}
        className="w-full py-3 mt-6 border-2 border-cyber-cyan text-cyber-cyan uppercase tracking-widest font-display"
      >
        Entsperren
      </button>
    </div>
  );
}

function MainPanel({
  pubkey,
  network,
  solLamports,
  tokenBalances,
  refreshing,
  onRefresh,
}: {
  pubkey: string;
  network: Net;
  solLamports: number;
  tokenBalances: Record<string, number>;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  const [sendOpen, setSendOpen] = useState(false);
  const [receiveOpen, setReceiveOpen] = useState(false);
  const [swapOpen, setSwapOpen] = useState(false);

  const solscanUrl = useMemo(
    () =>
      network === "devnet"
        ? `https://solscan.io/account/${pubkey}?cluster=devnet`
        : `https://solscan.io/account/${pubkey}`,
    [pubkey, network],
  );

  async function copyAddr() {
    try {
      await writeText(pubkey);
    } catch {
      // Fallback if clipboard plugin missing
      navigator.clipboard?.writeText(pubkey);
    }
  }

  return (
    <div className="max-w-2xl mx-auto">
      {/* Address card */}
      <div className="border border-cyber-cyan/40 p-4 mb-4 flex items-center justify-between">
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 border-2 border-cyber-cyan flex items-center justify-center font-bold text-cyber-cyan font-display">
            A
          </div>
          <div>
            <div className="text-cyber-cyan font-mono">{shortAddr(pubkey)}</div>
            <div className="text-soft-grey text-[10px] uppercase tracking-widest">
              {network}
            </div>
          </div>
        </div>
        <div className="flex gap-2">
          <button
            onClick={copyAddr}
            className="text-soft-grey hover:text-cyber-cyan text-xs px-2 py-1 border border-dim-green/40"
            title="Adresse kopieren"
          >
            COPY
          </button>
          <a
            href={solscanUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="text-soft-grey hover:text-cyber-cyan text-xs px-2 py-1 border border-dim-green/40"
          >
            SOLSCAN
          </a>
        </div>
      </div>

      {/* Balance */}
      <div className="border-2 border-cyber-cyan p-6 mb-4 pc-brand-glow text-center">
        <div className="text-soft-grey text-xs uppercase tracking-widest mb-1">
          Guthaben
        </div>
        <div className="flex items-baseline justify-center gap-2">
          <span className="text-cyber-cyan font-display text-4xl font-bold">
            {solHuman(solLamports)}
          </span>
          <span className="text-soft-grey uppercase tracking-widest">SOL</span>
        </div>
        {refreshing && <div className="text-cyber-cyan text-xs mt-2">⟳ refresh…</div>}
      </div>

      {/* Action bar */}
      <div className="grid grid-cols-3 gap-2 mb-6">
        <ActionBtn label="SENDEN" icon="↑" onClick={() => setSendOpen(true)} />
        <ActionBtn label="EMPFANGEN" icon="↓" onClick={() => setReceiveOpen(true)} />
        <ActionBtn label="SWAP" icon="⇄" onClick={() => setSwapOpen(true)} />
      </div>

      {/* Token list */}
      <div className="text-soft-grey text-xs uppercase tracking-widest mb-2">Tokens</div>
      {KNOWN_TOKENS.map((t) => {
        const bal = tokenBalances[t.mint] ?? 0;
        return (
          <div
            key={t.mint}
            className="border border-cyber-cyan/40 px-4 py-3 mb-2 flex items-center justify-between"
          >
            <div className="flex items-center gap-3">
              <div className="w-8 h-8 border border-cyber-cyan flex items-center justify-center text-cyber-cyan font-bold font-display">
                {t.symbol.slice(0, 1)}
              </div>
              <div>
                <div className="text-neon-green font-display tracking-widest">
                  {t.symbol}
                </div>
                <div className="text-soft-grey text-[10px]">{t.name}</div>
              </div>
            </div>
            <div
              className={
                "font-mono font-bold " +
                (bal > 0 ? "text-cyber-cyan" : "text-soft-grey")
              }
            >
              {tokenHuman(bal, t.decimals)}
            </div>
          </div>
        );
      })}

      <button
        onClick={onRefresh}
        className="w-full py-2 mt-4 text-soft-grey hover:text-cyber-cyan text-xs uppercase tracking-widest"
      >
        ⟳ Neu laden
      </button>

      {sendOpen && (
        <SendModal
          onClose={(refreshed) => {
            setSendOpen(false);
            if (refreshed) void onRefresh();
          }}
        />
      )}
      {receiveOpen && (
        <ReceiveModal pubkey={pubkey} onClose={() => setReceiveOpen(false)} />
      )}
      {swapOpen && (
        <SwapModal
          onClose={(refreshed) => {
            setSwapOpen(false);
            if (refreshed) void onRefresh();
          }}
        />
      )}
    </div>
  );
}

function ActionBtn({
  label,
  icon,
  onClick,
}: {
  label: string;
  icon: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="border-2 border-cyber-cyan py-3 text-cyber-cyan uppercase tracking-widest font-display hover:bg-cyber-cyan/10"
    >
      <div className="text-2xl mb-1">{icon}</div>
      <div className="text-[10px]">{label}</div>
    </button>
  );
}

function Modal({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  return (
    <div className="fixed inset-0 bg-black/70 z-50 flex items-center justify-center p-6">
      <div className="bg-bg-panel border-2 border-cyber-cyan max-w-md w-full max-h-[90vh] overflow-auto">
        <div className="flex justify-between items-center px-4 py-3 border-b border-dim-green/40">
          <h3 className="font-display tracking-widest text-cyber-cyan">{title}</h3>
          <button onClick={onClose} className="text-soft-grey hover:text-cyber-cyan">
            ✕
          </button>
        </div>
        <div className="p-4">{children}</div>
      </div>
    </div>
  );
}

function SendModal({ onClose }: { onClose: (refreshed: boolean) => void }) {
  const [asset, setAsset] = useState<string>("SOL");
  const [recipient, setRecipient] = useState("");
  const [amount, setAmount] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [sig, setSig] = useState<string | null>(null);

  async function send() {
    setBusy(true);
    setErr(null);
    try {
      const validated = await invoke<string>("argos_validate_address", {
        s: recipient,
      });
      const amt = parseFloat(amount.replace(",", "."));
      if (!isFinite(amt) || amt <= 0) throw "Betrag ungültig.";
      let signature: string;
      if (asset === "SOL") {
        const lamports = Math.round(amt * 1_000_000_000);
        signature = await invoke<string>("argos_send_sol", {
          recipientB58: validated,
          lamports,
        });
      } else {
        const tok = KNOWN_TOKENS.find((t) => t.symbol === asset)!;
        const raw = Math.round(amt * Math.pow(10, tok.decimals));
        signature = await invoke<string>("argos_send_token", {
          mintB58: tok.mint,
          recipientB58: validated,
          amount: raw,
        });
      }
      setSig(signature);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title="SENDEN" onClose={() => onClose(false)}>
      {sig ? (
        <div className="text-center py-4">
          <div className="text-4xl text-neon-green mb-2">✓</div>
          <div className="font-display tracking-widest text-neon-green mb-3">GESENDET</div>
          <div className="font-mono text-xs text-cyber-cyan break-all">{sig}</div>
          <button
            onClick={() => onClose(true)}
            className="w-full mt-4 py-3 border-2 border-cyber-cyan text-cyber-cyan uppercase tracking-widest"
          >
            Fertig
          </button>
        </div>
      ) : (
        <>
          <div className="flex gap-2 mb-3">
            {["SOL", ...KNOWN_TOKENS.map((t) => t.symbol)].map((s) => (
              <button
                key={s}
                onClick={() => setAsset(s)}
                className={
                  "flex-1 py-2 border text-xs uppercase tracking-widest " +
                  (asset === s
                    ? "border-cyber-cyan bg-cyber-cyan/10 text-cyber-cyan"
                    : "border-dim-green/40 text-soft-grey")
                }
              >
                {s}
              </button>
            ))}
          </div>
          <input
            value={recipient}
            onChange={(e) => setRecipient(e.target.value)}
            placeholder="Empfänger Solana-Adresse"
            className="w-full px-3 py-2 mb-2 bg-bg-deep border border-dim-green/40 text-cyber-cyan font-mono text-sm"
          />
          <input
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            placeholder={`Betrag ${asset}`}
            className="w-full px-3 py-2 bg-bg-deep border border-dim-green/40 text-cyber-cyan tracking-widest text-lg"
          />
          {err && <p className="text-neon-magenta text-xs mt-2">{err}</p>}
          <button
            disabled={busy}
            onClick={send}
            className="w-full py-3 mt-4 border-2 border-cyber-cyan text-cyber-cyan disabled:opacity-30 uppercase tracking-widest font-display"
          >
            {busy ? "Senden…" : `${asset} senden`}
          </button>
        </>
      )}
    </Modal>
  );
}

function ReceiveModal({ pubkey, onClose }: { pubkey: string; onClose: () => void }) {
  const qrSrc = `https://api.qrserver.com/v1/create-qr-code/?size=320x320&data=${encodeURIComponent(pubkey)}`;
  return (
    <Modal title="EMPFANGEN" onClose={onClose}>
      <div className="text-center">
        <div className="bg-white p-3 inline-block mb-3">
          <img src={qrSrc} alt="QR" width={280} height={280} />
        </div>
        <div className="font-mono text-xs text-cyber-cyan break-all px-2 mb-3">{pubkey}</div>
        <button
          onClick={() => navigator.clipboard.writeText(pubkey)}
          className="w-full py-3 border-2 border-cyber-cyan text-cyber-cyan uppercase tracking-widest"
        >
          ⎘ Adresse kopieren
        </button>
      </div>
    </Modal>
  );
}

function SwapModal({ onClose }: { onClose: (refreshed: boolean) => void }) {
  const [inAsset, setInAsset] = useState("SOL");
  const [outAsset, setOutAsset] = useState("USDC");
  const [amount, setAmount] = useState("");
  const [slippage, setSlippage] = useState(50);
  const [preview, setPreview] = useState<SwapPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [autoSend, setAutoSend] = useState(false);
  const [recipient, setRecipient] = useState("");
  const [sig, setSig] = useState<string | null>(null);

  function mintFor(s: string) {
    if (s === "SOL") return WSOL_MINT;
    return KNOWN_TOKENS.find((t) => t.symbol === s)!.mint;
  }
  function decFor(s: string) {
    if (s === "SOL") return 9;
    return KNOWN_TOKENS.find((t) => t.symbol === s)!.decimals;
  }

  async function quote() {
    setBusy(true);
    setErr(null);
    setPreview(null);
    try {
      const amt = parseFloat(amount.replace(",", "."));
      if (!isFinite(amt) || amt <= 0) throw "Betrag ungültig.";
      const raw = Math.round(amt * Math.pow(10, decFor(inAsset)));
      const p = await invoke<SwapPreview>("argos_quote_swap", {
        inputMintB58: mintFor(inAsset),
        outputMintB58: mintFor(outAsset),
        amountIn: raw,
        slippageBps: slippage,
      });
      setPreview(p);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function execute() {
    setBusy(true);
    setErr(null);
    try {
      let signature: string;
      if (autoSend) {
        if (!recipient.trim()) throw "Empfänger fehlt.";
        const outcome = await invoke<{ signature_b58: string }>(
          "argos_swap_and_send",
          { recipientB58: recipient.trim() },
        );
        signature = outcome.signature_b58;
      } else {
        signature = await invoke<string>("argos_execute_swap");
      }
      setSig(signature);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  function humanOut(raw: number) {
    const d = decFor(outAsset);
    return tokenHuman(raw, d) + " " + outAsset;
  }

  if (sig) {
    return (
      <Modal title="SWAP" onClose={() => onClose(true)}>
        <div className="text-center py-4">
          <div className="text-4xl text-neon-green mb-2">✓</div>
          <div className="font-display tracking-widest text-neon-green mb-3">
            {autoSend ? "GESCHWAPT & GESENDET" : "SWAP AUSGEFÜHRT"}
          </div>
          <div className="font-mono text-xs text-cyber-cyan break-all">{sig}</div>
        </div>
      </Modal>
    );
  }

  const tokens = ["SOL", ...KNOWN_TOKENS.map((t) => t.symbol)];
  return (
    <Modal title="SWAP · JUPITER v6" onClose={() => onClose(false)}>
      <p className="text-soft-grey text-[10px] mb-3">Gebühr: 0,5 % an Argos-Treasury</p>
      <div className="grid grid-cols-2 gap-2 mb-3">
        <select
          value={inAsset}
          onChange={(e) => {
            setInAsset(e.target.value);
            setPreview(null);
          }}
          className="px-3 py-2 bg-bg-deep border border-dim-green/40 text-cyber-cyan"
        >
          {tokens.map((s) => (
            <option key={s} value={s}>VON {s}</option>
          ))}
        </select>
        <select
          value={outAsset}
          onChange={(e) => {
            setOutAsset(e.target.value);
            setPreview(null);
          }}
          className="px-3 py-2 bg-bg-deep border border-dim-green/40 text-cyber-cyan"
        >
          {tokens.map((s) => (
            <option key={s} value={s}>ZU {s}</option>
          ))}
        </select>
      </div>
      <input
        value={amount}
        onChange={(e) => {
          setAmount(e.target.value);
          setPreview(null);
        }}
        placeholder={`Betrag ${inAsset}`}
        className="w-full px-3 py-2 bg-bg-deep border border-dim-green/40 text-cyber-cyan tracking-widest text-lg mb-2"
      />
      <div className="flex items-center justify-between mb-3">
        <span className="text-soft-grey text-xs">Slippage</span>
        <div className="flex gap-1">
          {[25, 50, 100, 300].map((bps) => (
            <button
              key={bps}
              onClick={() => {
                setSlippage(bps);
                setPreview(null);
              }}
              className={
                "px-2 py-1 text-xs border " +
                (slippage === bps
                  ? "border-cyber-cyan bg-cyber-cyan/10 text-cyber-cyan"
                  : "border-dim-green/40 text-soft-grey")
              }
            >
              {bps / 100}%
            </button>
          ))}
        </div>
      </div>
      <label className="flex items-center gap-2 mb-3 text-xs cursor-pointer">
        <input type="checkbox" checked={autoSend} onChange={(e) => setAutoSend(e.target.checked)} />
        <span className="text-cyber-cyan tracking-widest">Auto-Swap-on-Send (atomar)</span>
      </label>
      {autoSend && (
        <input
          value={recipient}
          onChange={(e) => setRecipient(e.target.value)}
          placeholder="Empfänger Solana-Adresse"
          className="w-full px-3 py-2 mb-3 bg-bg-deep border border-dim-green/40 text-cyber-cyan font-mono text-sm"
        />
      )}
      {preview && (
        <div className="border border-cyber-cyan/40 p-3 mb-3 text-xs">
          <div className="flex justify-between mb-1">
            <span className="text-soft-grey">Du erhältst</span>
            <span className="text-cyber-cyan font-display">
              {humanOut(preview.amount_out_expected)}
            </span>
          </div>
          <div className="flex justify-between mb-1">
            <span className="text-soft-grey">Mindestens</span>
            <span>{humanOut(preview.amount_out_min)}</span>
          </div>
          <div className="flex justify-between mb-1">
            <span className="text-soft-grey">Route</span>
            <span className="text-cyber-cyan">{preview.route_label}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-soft-grey">Argos-Gebühr</span>
            <span className="text-neon-magenta">{humanOut(preview.platform_fee_out)}</span>
          </div>
        </div>
      )}
      {err && <p className="text-neon-magenta text-xs mt-1 mb-2">{err}</p>}
      <button
        disabled={busy}
        onClick={preview ? execute : quote}
        className="w-full py-3 border-2 border-cyber-cyan text-cyber-cyan disabled:opacity-30 uppercase tracking-widest font-display"
      >
        {busy ? "…" : preview ? (autoSend ? "Swap & Send" : "Swap ausführen") : "Preview holen"}
      </button>
    </Modal>
  );
}
