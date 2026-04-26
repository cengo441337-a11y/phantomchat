import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { VoiceMeta } from "../types";

interface Props {
  /// Voice metadata pulled off `MsgLine.voice_meta`. Carries the codec
  /// hint, duration_ms (sender-stamped, used for the label until
  /// `loadedmetadata` fires) and the absolute on-disk path the desktop
  /// saved the audio bytes to.
  meta: VoiceMeta;
  /// Whether this is an outgoing-side echo. Outgoing voice rows get a
  /// neon-green tint to mirror the rest of the message stream's colour
  /// vocabulary; incoming rows render in cyber-cyan. The desktop doesn't
  /// record voice yet (Wave 11B mobile-only), but the prop is wired up so
  /// the future desktop-record path can immediately re-use this bubble.
  outgoing?: boolean;
}

/// Format milliseconds as `M:SS` (or `H:MM:SS` if ≥ 1h). Used for the
/// duration-out-of-total label.
function formatMs(ms: number): string {
  const totalSecs = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(totalSecs / 3600);
  const mins = Math.floor((totalSecs % 3600) / 60);
  const secs = totalSecs % 60;
  if (hours > 0) {
    return `${hours}:${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
  }
  return `${mins}:${String(secs).padStart(2, "0")}`;
}

/// VoiceMessageBubble — renders a play / pause button, a progress bar,
/// the live elapsed / total label, and a small codec badge. Uses the
/// HTML5 `<audio>` element under the hood; both `opus` (in `.ogg`) and
/// `aac` (in `.m4a`) decode natively in the Tauri webview without an
/// extra Rust dep. The on-disk path is fed through `convertFileSrc` to
/// produce the `tauri://` URL the element fetches.
export default function VoiceMessageBubble({ meta, outgoing }: Props) {
  const { t } = useTranslation();
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [playing, setPlaying] = useState(false);
  const [elapsedMs, setElapsedMs] = useState(0);
  /// Total duration: prefer the `<audio>` element's reported value once
  /// it loads (most accurate), fall back to the sender-stamped
  /// `meta.duration_ms` until then so the bubble always shows a label.
  const [actualDurationMs, setActualDurationMs] = useState<number | null>(null);
  const [errored, setErrored] = useState(false);

  // Build the asset URL once. `convertFileSrc` is idempotent + cheap, but
  // we still memoise via state so a re-render doesn't churn the <audio>
  // element's `src` (which would re-trigger the network/disk fetch).
  const src = convertFileSrc(meta.path);

  // Wire up the audio element's lifecycle event handlers. We rely on
  // `timeupdate` for the progress bar; React state is intentionally
  // updated at ~4Hz (browser default) which is plenty smooth for the
  // 200px-wide progress track.
  useEffect(() => {
    const el = audioRef.current;
    if (!el) return;
    const onTime = () => setElapsedMs(Math.floor((el.currentTime || 0) * 1000));
    const onLoaded = () => {
      // `<audio>.duration` is in seconds; some opus files report
      // `Infinity` until a full play-through, in which case we keep the
      // sender-stamped duration as the source of truth.
      if (Number.isFinite(el.duration) && el.duration > 0) {
        setActualDurationMs(Math.floor(el.duration * 1000));
      }
    };
    const onPlay = () => setPlaying(true);
    const onPause = () => setPlaying(false);
    const onEnded = () => {
      setPlaying(false);
      setElapsedMs(actualDurationMs ?? meta.duration_ms);
    };
    const onError = () => {
      setErrored(true);
      setPlaying(false);
    };
    el.addEventListener("timeupdate", onTime);
    el.addEventListener("loadedmetadata", onLoaded);
    el.addEventListener("play", onPlay);
    el.addEventListener("pause", onPause);
    el.addEventListener("ended", onEnded);
    el.addEventListener("error", onError);
    return () => {
      el.removeEventListener("timeupdate", onTime);
      el.removeEventListener("loadedmetadata", onLoaded);
      el.removeEventListener("play", onPlay);
      el.removeEventListener("pause", onPause);
      el.removeEventListener("ended", onEnded);
      el.removeEventListener("error", onError);
    };
    // `actualDurationMs` is captured by `onEnded` for the snap-to-end
    // behaviour; re-run the effect whenever it changes so the closure
    // stays consistent.
  }, [actualDurationMs, meta.duration_ms]);

  const totalMs = actualDurationMs ?? meta.duration_ms;
  const progress = totalMs > 0 ? Math.min(100, (elapsedMs / totalMs) * 100) : 0;

  function togglePlay() {
    const el = audioRef.current;
    if (!el || errored) return;
    if (el.paused) {
      void el.play().catch(() => setErrored(true));
    } else {
      el.pause();
    }
  }

  const tint = outgoing ? "text-neon-green" : "text-cyber-cyan";
  const trackColor = outgoing ? "bg-neon-green/40" : "bg-cyber-cyan/40";
  const fillColor = outgoing ? "bg-neon-green" : "bg-cyber-cyan";

  return (
    <span
      className={
        "inline-flex items-center gap-2 rounded border border-dim-green/40 bg-bg-elevated px-2 py-1 text-xs " +
        tint
      }
    >
      <button
        onClick={togglePlay}
        disabled={errored}
        aria-label={playing ? t("voice.pause") : t("voice.play")}
        title={playing ? t("voice.pause") : t("voice.play")}
        className={
          "shrink-0 w-6 h-6 rounded-full flex items-center justify-center text-sm leading-none border transition-colors " +
          (errored
            ? "border-red-500/60 text-red-300 cursor-not-allowed"
            : "border-dim-green/60 hover:border-neon-magenta/60 hover:text-neon-magenta")
        }
      >
        {errored ? "!" : playing ? "⏸" : "▶"}
      </button>
      <span aria-hidden="true" className="text-base leading-none">
        {"\u{1F399}"}
      </span>
      <div
        className={"relative h-1.5 w-[140px] rounded-full overflow-hidden " + trackColor}
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={totalMs}
        aria-valuenow={Math.min(elapsedMs, totalMs)}
      >
        <div
          className={"h-full " + fillColor}
          style={{ width: `${progress}%`, transition: "width 120ms linear" }}
        />
      </div>
      <span className="font-mono text-[11px] text-soft-grey tabular-nums">
        {formatMs(elapsedMs)} / {formatMs(totalMs)}
      </span>
      <span className="text-[10px] uppercase tracking-wider text-soft-grey/80">
        {meta.codec}
      </span>
      {errored && (
        <span className="text-red-300 text-[10px]" title={t("voice.load_error")}>
          {t("voice.load_error")}
        </span>
      )}
      {/* The actual <audio> element. Hidden — we drive playback via the
          custom button above. preload="metadata" so duration is available
          without burning bandwidth on the full audio bytes upfront. */}
      <audio
        ref={audioRef}
        src={src}
        preload="metadata"
        style={{ display: "none" }}
      />
    </span>
  );
}
