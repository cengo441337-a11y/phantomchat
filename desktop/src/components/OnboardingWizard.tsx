import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import AddressQR from "./AddressQR";
import ThemeSwitcher from "./ThemeSwitcher";

interface Props {
  onDone: () => void;
}

/// Total wizard steps. Bumped from 5 → 6 in wave-8d when we inserted the
/// "Erscheinungsbild wählen" step between step 5 (share address) and the
/// final "Done" step. Renumbering the trailing step (and widening every
/// `1 | 2 | 3 | 4 | 5` union accordingly) keeps the flow's tail logic
/// intact.
const TOTAL_STEPS = 7;

type StepIdx = 1 | 2 | 3 | 4 | 5 | 6 | 7;

const DEFAULT_RELAY_OPTIONS = [
  "wss://relay.damus.io",
  "wss://nos.lol",
  "wss://relay.snort.social",
];

/// First-launch wizard. Runs only when `is_onboarded` returns false.
///
/// Steps:
///   1. Welcome           — splash + Continue
///   2. Identity          — Generate OR Restore (paste keys.json)
///   3. Join LAN org?     — optional zero-touch mDNS discovery (skip / join / create)
///   4. Relays            — checkboxes + custom URL → set_relays
///   5. Share address     — show user's address with Copy
///   6. Done              — Start (mark_onboarded)
///
/// State is local; the only side-effects that persist are
/// generate_identity / import_keyfile / lan_org_create / lan_org_join /
/// set_relays / mark_onboarded.
export default function OnboardingWizard({ onDone }: Props) {
  const { t } = useTranslation();
  const [step, setStep] = useState<StepIdx>(1);

  // ── Step 2 ──────────────────────────────────────────────────────────────
  const [identityMode, setIdentityMode] = useState<"none" | "generate" | "restore">(
    "none",
  );
  const [restoreText, setRestoreText] = useState("");
  const [identityBusy, setIdentityBusy] = useState(false);
  const [identityErr, setIdentityErr] = useState<string | null>(null);
  const [identityDone, setIdentityDone] = useState(false);

  // ── Step 3 (LAN org) ────────────────────────────────────────────────────
  // Three exclusive choices on this step:
  //   - "skip"    : user clicks "Skip" → step is satisfied without a daemon
  //   - "join"    : user pastes a 6-char code, lan_org_join succeeds
  //   - "create"  : user clicks "Create new LAN org" → lan_org_create returns
  //                 a code we render with a Copy button.
  // The user can change their mind: clicking Join after Create resets state.
  const [lanMode, setLanMode] = useState<"none" | "join" | "create">("none");
  const [lanCodeInput, setLanCodeInput] = useState("");
  const [lanBusy, setLanBusy] = useState(false);
  const [lanErr, setLanErr] = useState<string | null>(null);
  const [lanDone, setLanDone] = useState(false);
  const [lanCodeFromCreate, setLanCodeFromCreate] = useState<string | null>(null);
  const [lanCopied, setLanCopied] = useState(false);

  // ── Step 4 (Relays) ─────────────────────────────────────────────────────
  const [selectedDefaults, setSelectedDefaults] = useState<Set<string>>(
    new Set(DEFAULT_RELAY_OPTIONS),
  );
  const [customRelay, setCustomRelay] = useState("");
  const [relaysBusy, setRelaysBusy] = useState(false);
  const [relaysErr, setRelaysErr] = useState<string | null>(null);
  const [relaysDone, setRelaysDone] = useState(false);

  // ── Step 5 ──────────────────────────────────────────────────────────────
  const [address, setAddress] = useState<string | null>(null);
  const [qrSvg, setQrSvg] = useState<string | null>(null);

  // ── Step 6 (final) ──────────────────────────────────────────────────────
  // ── Step 6 ──────────────────────────────────────────────────────────────
  const [finishing, setFinishing] = useState(false);
  const [finishErr, setFinishErr] = useState<string | null>(null);

  // Pull the address + QR as soon as identity is done so step 4 has them
  // ready. The QR is best-effort: if encoding fails for any reason, the
  // AddressQR component falls back to its "loading qr…" placeholder
  // while the plain text address is still shown below.
  useEffect(() => {
    if (!identityDone) return;
    (async () => {
      try {
        const a = await invoke<string>("get_address");
        setAddress(a);
      } catch {
        /* user will see "..." in step 4 — acceptable */
      }
      try {
        const svg = await invoke<string>("address_qr_svg");
        setQrSvg(svg);
      } catch {
        /* leave qr null */
      }
    })();
  }, [identityDone]);

  async function doGenerate() {
    setIdentityBusy(true);
    setIdentityErr(null);
    try {
      await invoke<{ address: string }>("generate_identity");
      setIdentityDone(true);
    } catch (e) {
      setIdentityErr(String(e));
    } finally {
      setIdentityBusy(false);
    }
  }

  async function doRestore() {
    setIdentityBusy(true);
    setIdentityErr(null);
    try {
      await invoke("import_keyfile", { jsonText: restoreText });
      setIdentityDone(true);
    } catch (e) {
      setIdentityErr(String(e));
    } finally {
      setIdentityBusy(false);
    }
  }

  /// Auto-uppercase + auto-hyphen for the LAN-org code field. Strips
  /// any user-typed hyphens, uppercases, then re-inserts a hyphen after
  /// the third char so the input always renders as `XXX-XXX`. Caps at
  /// 7 characters (6 alphanumerics + 1 hyphen).
  function formatLanCode(raw: string): string {
    const cleaned = raw.replace(/[^0-9A-Za-z]/g, "").toUpperCase().slice(0, 6);
    if (cleaned.length <= 3) return cleaned;
    return `${cleaned.slice(0, 3)}-${cleaned.slice(3)}`;
  }

  async function doLanJoin() {
    setLanBusy(true);
    setLanErr(null);
    try {
      await invoke("lan_org_join", { code: lanCodeInput });
      setLanDone(true);
    } catch (e) {
      setLanErr(String(e));
    } finally {
      setLanBusy(false);
    }
  }

  async function doLanCreate() {
    setLanBusy(true);
    setLanErr(null);
    try {
      const code = await invoke<string>("lan_org_create");
      setLanCodeFromCreate(code);
      setLanDone(true);
    } catch (e) {
      setLanErr(String(e));
    } finally {
      setLanBusy(false);
    }
  }

  function lanSkip() {
    // Skip path: no daemon spawned, just mark this step satisfied.
    setLanMode("none");
    setLanDone(true);
    setLanErr(null);
  }

  async function copyLanCode() {
    if (!lanCodeFromCreate) return;
    try {
      await navigator.clipboard.writeText(lanCodeFromCreate);
      setLanCopied(true);
      window.setTimeout(() => setLanCopied(false), 1200);
    } catch {
      /* clipboard refused — silent */
    }
  }

  function toggleDefault(url: string) {
    setSelectedDefaults(prev => {
      const next = new Set(prev);
      if (next.has(url)) next.delete(url);
      else next.add(url);
      return next;
    });
  }

  async function saveRelays() {
    setRelaysBusy(true);
    setRelaysErr(null);
    try {
      const list: string[] = Array.from(selectedDefaults);
      const custom = customRelay.trim();
      if (custom.length > 0) list.push(custom);
      if (list.length === 0) {
        setRelaysErr(t("onboarding.step3.pick_at_least_one"));
        return;
      }
      await invoke("set_relays", { urls: list });
      setRelaysDone(true);
    } catch (e) {
      setRelaysErr(String(e));
    } finally {
      setRelaysBusy(false);
    }
  }

  async function finish() {
    setFinishing(true);
    setFinishErr(null);
    try {
      await invoke("mark_onboarded");
      onDone();
    } catch (e) {
      setFinishErr(String(e));
      setFinishing(false);
    }
  }

  // Forward-button enablement reflects per-step prerequisites.
  function canAdvance(): boolean {
    switch (step) {
      case 1:
        return true;
      case 2:
        return identityDone;
      case 3:
        return lanDone;
      case 4:
        return relaysDone;
      case 5:
        return true; // share-address step — always advanceable.
      case 6:
        return true; // theme step — any default is valid, picker is live.
      case 7:
        return false; // final step uses the dedicated "Start" button.
    }
  }

  function back() {
    if (step > 1) setStep((s) => (s - 1) as StepIdx);
  }
  function next() {
    if (step < TOTAL_STEPS && canAdvance()) {
      setStep((s) => (s + 1) as StepIdx);
    }
  }

  return (
    <div className="h-screen w-screen flex items-center justify-center bg-bg-deep text-neon-green font-mono">
      <div className="max-w-xl w-full p-8 panel-border-active space-y-5">
        {/* Progress dots */}
        <div className="flex justify-center gap-2 pb-2">
          {Array.from({ length: TOTAL_STEPS }, (_, i) => i + 1).map(n => (
            <span
              key={n}
              className={
                "inline-block w-2.5 h-2.5 rounded-full border " +
                (n === step
                  ? "bg-neon-green border-neon-green shadow-neon-green"
                  : n < step
                  ? "bg-neon-green/60 border-neon-green/60"
                  : "bg-transparent border-dim-green/60")
              }
              aria-label={`step ${n}${n === step ? " (current)" : ""}`}
            />
          ))}
        </div>

        {/* Step body */}
        {step === 1 && (
          <div className="text-center space-y-5">
            <div className="text-3xl tracking-widest font-bold">
              PHANTOM<span className="text-neon-magenta">CHAT</span>
            </div>
            <div className="text-xs text-soft-grey uppercase tracking-widest">
              {t("onboarding.step1.subtitle")}
            </div>
            <p className="text-sm text-soft-grey leading-relaxed">
              {t("onboarding.step1.description")}
            </p>
          </div>
        )}

        {step === 2 && (
          <div className="space-y-4">
            <div className="text-center text-xs text-soft-grey uppercase tracking-widest">
              {t("onboarding.step2.header")}
            </div>
            {!identityDone ? (
              <>
                <div className="flex gap-2">
                  <button
                    onClick={() => setIdentityMode("generate")}
                    className={
                      "flex-1 neon-button " +
                      (identityMode === "generate"
                        ? "shadow-neon-green"
                        : "")
                    }
                  >
                    {t("onboarding.step2.generate_new")}
                  </button>
                  <button
                    onClick={() => setIdentityMode("restore")}
                    className={
                      "flex-1 neon-button " +
                      (identityMode === "restore" ? "shadow-neon-green" : "")
                    }
                  >
                    {t("onboarding.step2.restore_from_keyfile")}
                  </button>
                </div>

                {identityMode === "generate" && (
                  <div className="space-y-3">
                    <p className="text-xs text-soft-grey">
                      {t("onboarding.step2.generate_description")}
                    </p>
                    <button
                      onClick={() => void doGenerate()}
                      disabled={identityBusy}
                      className="neon-button w-full disabled:opacity-40"
                    >
                      {identityBusy
                        ? t("onboarding.step2.generating")
                        : t("onboarding.step2.generate_button")}
                    </button>
                  </div>
                )}

                {identityMode === "restore" && (
                  <div className="space-y-3">
                    <p className="text-xs text-soft-grey">
                      {t("onboarding.step2.restore_description")}
                    </p>
                    <textarea
                      value={restoreText}
                      onChange={e => setRestoreText(e.target.value)}
                      placeholder='{"view_private":"…","view_public":"…", … }'
                      className="neon-input w-full h-40 text-xs resize-none"
                    />
                    <button
                      onClick={() => void doRestore()}
                      disabled={identityBusy || restoreText.trim().length === 0}
                      className="neon-button w-full disabled:opacity-40"
                    >
                      {identityBusy
                        ? t("onboarding.step2.importing")
                        : t("onboarding.step2.import_button")}
                    </button>
                  </div>
                )}

                {identityErr && (
                  <div className="text-xs text-neon-magenta border border-neon-magenta/50 rounded-md p-2 break-words">
                    {identityErr}
                  </div>
                )}
              </>
            ) : (
              <div className="text-center text-sm text-neon-green">
                {t("onboarding.step2.ready_message")}
              </div>
            )}
          </div>
        )}

        {step === 3 && (
          <div className="space-y-4">
            <div className="text-center text-xs text-soft-grey uppercase tracking-widest">
              {t("onboarding.lan.header")}
            </div>
            <p className="text-xs text-soft-grey leading-relaxed">
              {t("onboarding.lan.description")}
            </p>

            {/* Mode-selector buttons — three exclusive choices. */}
            {!lanDone && (
              <div className="flex flex-col gap-2">
                <button
                  onClick={lanSkip}
                  disabled={lanBusy}
                  className="neon-button w-full text-xs disabled:opacity-40"
                >
                  {t("onboarding.lan.skip_button")}
                </button>
                <button
                  onClick={() => {
                    setLanMode("join");
                    setLanCodeFromCreate(null);
                    setLanErr(null);
                  }}
                  disabled={lanBusy}
                  className={
                    "neon-button w-full text-xs disabled:opacity-40 " +
                    (lanMode === "join" ? "shadow-neon-green" : "")
                  }
                >
                  {t("onboarding.lan.join_button")}
                </button>
                <button
                  onClick={() => {
                    setLanMode("create");
                    setLanCodeInput("");
                    setLanErr(null);
                    void doLanCreate();
                  }}
                  disabled={lanBusy}
                  className={
                    "neon-button w-full text-xs disabled:opacity-40 " +
                    (lanMode === "create" ? "shadow-neon-green" : "")
                  }
                >
                  {lanBusy && lanMode === "create"
                    ? t("onboarding.lan.creating")
                    : t("onboarding.lan.create_button")}
                </button>
              </div>
            )}

            {/* Join-with-code input. Visible once the user picks "join". */}
            {!lanDone && lanMode === "join" && (
              <div className="space-y-2 pt-1">
                <input
                  type="text"
                  value={lanCodeInput}
                  onChange={e => setLanCodeInput(formatLanCode(e.target.value))}
                  placeholder={t("onboarding.lan.code_placeholder")}
                  className="neon-input w-full text-xs font-mono tracking-widest"
                  maxLength={7}
                />
                <button
                  onClick={() => void doLanJoin()}
                  disabled={lanBusy || lanCodeInput.replace("-", "").length !== 6}
                  className="neon-button w-full text-xs disabled:opacity-40"
                >
                  {lanBusy
                    ? t("onboarding.lan.joining")
                    : t("onboarding.lan.confirm_join_button")}
                </button>
              </div>
            )}

            {/* Create-side: show the assigned code + Copy button. */}
            {lanDone && lanCodeFromCreate && (
              <div className="space-y-2 pt-1 text-center">
                <div className="text-[10px] text-soft-grey uppercase tracking-widest">
                  {t("onboarding.lan.your_code_label")}
                </div>
                <div className="text-3xl font-mono font-bold text-neon-green tracking-widest select-all">
                  {lanCodeFromCreate}
                </div>
                <button
                  onClick={() => void copyLanCode()}
                  className="neon-button text-xs"
                >
                  {lanCopied
                    ? t("onboarding.lan.copied_button")
                    : t("onboarding.lan.copy_button")}
                </button>
                <p className="text-xs text-soft-grey">
                  {t("onboarding.lan.share_with_colleagues")}
                </p>
              </div>
            )}

            {/* Discovery-in-background hint (after a successful join). */}
            {lanDone && !lanCodeFromCreate && lanMode === "join" && (
              <div className="text-xs text-cyber-cyan text-center">
                {t("onboarding.lan.discovering_in_background")}
              </div>
            )}

            {/* Skip-confirmation banner so the user knows they advanced. */}
            {lanDone && lanMode === "none" && (
              <div className="text-xs text-soft-grey text-center italic">
                {t("onboarding.lan.skipped_message")}
              </div>
            )}

            {lanErr && (
              <div className="text-xs text-neon-magenta border border-neon-magenta/50 rounded-md p-2 break-words">
                {lanErr}
              </div>
            )}

            {/* Privacy / threat-model warning — magenta-bordered. */}
            <div className="text-[10px] text-neon-magenta border border-neon-magenta/60 rounded-md p-2 leading-relaxed">
              {t("onboarding.lan.warning")}
            </div>
          </div>
        )}

        {step === 4 && (
          <div className="space-y-4">
            <div className="text-center text-xs text-soft-grey uppercase tracking-widest">
              {t("onboarding.step3.header")}
            </div>
            <p className="text-xs text-soft-grey">
              {t("onboarding.step3.description")}
            </p>
            <div className="space-y-2">
              {DEFAULT_RELAY_OPTIONS.map(url => (
                <label
                  key={url}
                  className="flex items-center gap-2 text-xs cursor-pointer"
                >
                  <input
                    type="checkbox"
                    checked={selectedDefaults.has(url)}
                    onChange={() => toggleDefault(url)}
                    className="accent-neon-green"
                  />
                  <span className="font-mono">{url}</span>
                </label>
              ))}
            </div>
            <div>
              <div className="text-xs text-soft-grey uppercase tracking-wider mb-1">
                {t("onboarding.step3.custom_label")}
              </div>
              <input
                type="text"
                value={customRelay}
                onChange={e => setCustomRelay(e.target.value)}
                placeholder="wss://your-relay.example.org"
                className="neon-input w-full text-xs"
              />
            </div>
            <button
              onClick={() => void saveRelays()}
              disabled={relaysBusy}
              className="neon-button w-full disabled:opacity-40"
            >
              {relaysBusy
                ? t("onboarding.step3.saving")
                : relaysDone
                ? t("onboarding.step3.saved_message")
                : t("onboarding.step3.save_button")}
            </button>
            {relaysErr && (
              <div className="text-xs text-neon-magenta border border-neon-magenta/50 rounded-md p-2 break-words">
                {relaysErr}
              </div>
            )}
          </div>
        )}

        {step === 5 && (
          <div className="space-y-4">
            <div className="text-center text-xs text-soft-grey uppercase tracking-widest">
              {t("onboarding.step4.header")}
            </div>
            <p className="text-xs text-soft-grey">
              {t("onboarding.step4.description")}
            </p>
            <AddressQR svg={qrSvg} address={address} />
          </div>
        )}

        {step === 6 && (
          <div className="text-center space-y-5">
            <div className="text-2xl tracking-widest font-bold text-neon-green">
              {t("onboarding.step5.header")}
            </div>
            <p className="text-xs text-soft-grey">
              {t("onboarding.step5.description")}
            </p>
            <ThemeSwitcher />
            <p className="text-[10px] text-soft-grey italic text-center">
              {t("onboarding.step5.hint")}
            </p>
          </div>
        )}

        {step === 7 && (
          <div className="text-center space-y-5">
            <div className="text-2xl tracking-widest font-bold text-neon-green">
              {t("onboarding.step6.header")}
            </div>
            <p className="text-sm text-soft-grey leading-relaxed">
              {t("onboarding.step6.description")}
            </p>
            <button
              onClick={() => void finish()}
              disabled={finishing}
              className="neon-button w-full text-base disabled:opacity-40"
            >
              {finishing
                ? t("onboarding.step6.starting")
                : t("onboarding.step6.start_button")}
            </button>
            {finishErr && (
              <div className="text-xs text-neon-magenta border border-neon-magenta/50 rounded-md p-2 break-words">
                {finishErr}
              </div>
            )}
          </div>
        )}

        {/* Back / Next bar (hidden on the final step — that step has its own
            "Start" button). */}
        {step !== TOTAL_STEPS && (
          <div className="flex justify-between pt-3 border-t border-dim-green/30">
            <button
              onClick={back}
              disabled={step === 1}
              className="text-soft-grey hover:text-neon-green text-sm uppercase tracking-wider px-3 py-2 disabled:opacity-30"
            >
              {t("onboarding.nav.back")}
            </button>
            <button
              onClick={next}
              disabled={!canAdvance()}
              className="neon-button text-sm disabled:opacity-40"
            >
              {t("onboarding.nav.next")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
