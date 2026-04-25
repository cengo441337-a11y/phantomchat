import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import AddressQR from "./AddressQR";

interface Props {
  onDone: () => void;
}

const TOTAL_STEPS = 5;

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
///   3. Relays            — checkboxes + custom URL → set_relays
///   4. Share address     — show user's address with Copy
///   5. Done              — Start (mark_onboarded)
///
/// State is local; the only side-effects that persist are
/// generate_identity / import_keyfile / set_relays / mark_onboarded.
export default function OnboardingWizard({ onDone }: Props) {
  const { t } = useTranslation();
  const [step, setStep] = useState<1 | 2 | 3 | 4 | 5>(1);

  // ── Step 2 ──────────────────────────────────────────────────────────────
  const [identityMode, setIdentityMode] = useState<"none" | "generate" | "restore">(
    "none",
  );
  const [restoreText, setRestoreText] = useState("");
  const [identityBusy, setIdentityBusy] = useState(false);
  const [identityErr, setIdentityErr] = useState<string | null>(null);
  const [identityDone, setIdentityDone] = useState(false);

  // ── Step 3 ──────────────────────────────────────────────────────────────
  const [selectedDefaults, setSelectedDefaults] = useState<Set<string>>(
    new Set(DEFAULT_RELAY_OPTIONS),
  );
  const [customRelay, setCustomRelay] = useState("");
  const [relaysBusy, setRelaysBusy] = useState(false);
  const [relaysErr, setRelaysErr] = useState<string | null>(null);
  const [relaysDone, setRelaysDone] = useState(false);

  // ── Step 4 ──────────────────────────────────────────────────────────────
  const [address, setAddress] = useState<string | null>(null);
  const [qrSvg, setQrSvg] = useState<string | null>(null);

  // ── Step 5 ──────────────────────────────────────────────────────────────
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
        return relaysDone;
      case 4:
        return true;
      case 5:
        return false; // step 5 uses the dedicated "Start" button.
    }
  }

  function back() {
    if (step > 1) setStep((s) => (s - 1) as 1 | 2 | 3 | 4 | 5);
  }
  function next() {
    if (step < 5 && canAdvance()) {
      setStep((s) => (s + 1) as 1 | 2 | 3 | 4 | 5);
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

        {step === 4 && (
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

        {step === 5 && (
          <div className="text-center space-y-5">
            <div className="text-2xl tracking-widest font-bold text-neon-green">
              {t("onboarding.step5.header")}
            </div>
            <p className="text-sm text-soft-grey leading-relaxed">
              {t("onboarding.step5.description")}
            </p>
            <button
              onClick={() => void finish()}
              disabled={finishing}
              className="neon-button w-full text-base disabled:opacity-40"
            >
              {finishing
                ? t("onboarding.step5.starting")
                : t("onboarding.step5.start_button")}
            </button>
            {finishErr && (
              <div className="text-xs text-neon-magenta border border-neon-magenta/50 rounded-md p-2 break-words">
                {finishErr}
              </div>
            )}
          </div>
        )}

        {/* Back / Next bar (hidden on step 5 — that step has its own button) */}
        {step !== 5 && (
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
