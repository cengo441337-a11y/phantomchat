import { useState } from "react";
import { useTranslation } from "react-i18next";

interface Props {
  /// Inline SVG markup returned by the `address_qr_svg` Tauri command.
  /// Expected to be a `<svg>…</svg>` string with one `<rect>` per dark
  /// module and no inline color styling — coloring is applied here via
  /// CSS so the QR matches the cyberpunk palette of the rest of the UI.
  svg: string | null;
  /// Plain `phantom:view:spend` address, used by the "copy address"
  /// button. Kept separate from `svg` so the parent can refresh either
  /// one without re-running the QR encode.
  address: string | null;
}

/// Inline-SVG QR renderer for the user's PhantomChat address.
///
/// The SVG comes pre-built from the Rust side (`address_qr_svg` Tauri
/// command) and is dropped into the DOM via `dangerouslySetInnerHTML`.
/// `[&_rect]:fill-neon-green` colors every emitted `<rect>` (i.e. every
/// dark module of the QR) in the brand neon-green; the surrounding
/// container provides the dark padding so a phone camera has enough
/// contrast to lock onto the finder patterns.
///
/// Below the QR, a "copy address" button writes the raw text address to
/// the clipboard — useful when the user is sharing over a chat / email
/// channel where a QR scan isn't an option.
export default function AddressQR({ svg, address }: Props) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  async function copy() {
    if (!address) return;
    try {
      await navigator.clipboard.writeText(address);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard unavailable — silently no-op */
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex justify-center">
        <div
          className="bg-bg-deep border border-dim-green/60 rounded-md p-3 w-56 h-56 flex items-center justify-center [&_svg]:w-full [&_svg]:h-full [&_rect]:fill-neon-green"
          aria-label={t("address_qr.aria")}
        >
          {svg ? (
            <div
              className="w-full h-full"
              dangerouslySetInnerHTML={{ __html: svg }}
            />
          ) : (
            <span className="text-xs text-soft-grey">{t("address_qr.loading")}</span>
          )}
        </div>
      </div>
      {address && (
        <div className="bg-bg-deep border border-dim-green/40 rounded-md p-2 break-all text-[11px] font-mono text-neon-green">
          {address}
        </div>
      )}
      <div className="flex justify-center">
        <button
          onClick={() => void copy()}
          disabled={!address}
          className="neon-button text-xs disabled:opacity-40"
        >
          {copied ? t("address_qr.copied") : t("address_qr.copy")}
        </button>
      </div>
    </div>
  );
}
