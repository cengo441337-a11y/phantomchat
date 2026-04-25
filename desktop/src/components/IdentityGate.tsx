import { useTranslation } from "react-i18next";

interface Props {
  onGenerate: () => void | Promise<void>;
  error: string | null;
}

export default function IdentityGate({ onGenerate, error }: Props) {
  const { t } = useTranslation();
  return (
    <div className="h-screen w-screen flex items-center justify-center bg-bg-deep text-neon-green font-mono">
      <div className="max-w-md w-full p-8 panel-border-active text-center space-y-5 bg-bg-panel/80 backdrop-blur-sm">
        <div className="text-3xl tracking-widest font-bold text-neon-green font-display pc-brand-glow">
          {t("identity_gate.title_a")}
          <span className="text-neon-magenta pc-brand-glow-magenta">
            {t("identity_gate.title_b")}
          </span>
        </div>
        <div className="text-xs text-soft-grey uppercase tracking-widest font-display">
          {t("identity_gate.subtitle")}
        </div>

        <p className="text-sm text-soft-grey leading-relaxed">
          {t("identity_gate.description")}
        </p>

        <button
          onClick={() => void onGenerate()}
          className="neon-button w-full text-base"
        >
          {t("identity_gate.generate_button")}
        </button>

        {error && (
          <div className="text-xs text-neon-magenta border border-neon-magenta/50 rounded-md p-2 break-words">
            {error}
          </div>
        )}
      </div>
    </div>
  );
}
