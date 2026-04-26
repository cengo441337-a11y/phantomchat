//! Wave 11D — on-device speech-to-text for incoming voice messages.
//!
//! Wired from `ai_bridge_maybe_handle` in `lib.rs` when an inbound message
//! starts with the `VOICE-1:` wire prefix and STT is enabled at runtime.
//! Transcription happens entirely on the home machine — even with a
//! cloud-LLM provider configured, the LLM only sees the resulting text,
//! never the raw audio bytes.
//!
//! ## Pipeline
//!
//! 1. `symphonia` opens the on-disk audio file (the Wave 11B receive path
//!    wrote it to `<app_data>/voice/<msg_id>.<ext>`). For Android API ≥ 29
//!    senders this is `.ogg` (Opus); older senders fall back to `.m4a`
//!    (AAC in ISO BMFF). symphonia 0.5 parses the OGG container natively
//!    but does NOT decode the Opus codec stream itself — opus payloads
//!    therefore surface as `unsupported codec`. Callers handle that as
//!    "skip auto-reply for this message" (audit-logged, never a panic).
//! 2. Decoded i16/i32/f32 samples → mono → 16 kHz f32 (linear-interp
//!    resampler — adequate for whisper's input requirements; `rubato`
//!    would improve quality but adds 2 MB to the binary for an
//!    inaudible win).
//! 3. `whisper-rs` runs `WhisperContext::full(...)` with the supplied
//!    language hint (or auto-detect when `None`).
//! 4. Concatenate every segment's text and return.
//!
//! Errors at any stage become `Err(anyhow!)` — the caller logs them and
//! skips the auto-reply, but the bridge stays alive.

#![cfg(feature = "stt")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, Context, Result};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Cached whisper context, keyed by the loaded model path. Hazard: the
/// previous code rebuilt `WhisperContext::new_with_params` (a 100-300 ms
/// mmap + GGML init) on every transcription. We cache the most recently
/// loaded model and evict + reload only when the configured `model_path`
/// changes between calls.
struct CachedWhisper {
    model_path: PathBuf,
    ctx: Arc<WhisperContext>,
}

fn whisper_cache() -> &'static Mutex<Option<CachedWhisper>> {
    static CACHE: OnceLock<Mutex<Option<CachedWhisper>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Whisper expects 16 kHz mono f32. Hard-coded — every model variant
/// (tiny → large) ingests at this rate, so there's no reason to thread
/// it through the config.
const WHISPER_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone)]
pub struct WhisperConfig {
    pub enabled: bool,
    pub model_path: PathBuf,
    /// `None` → whisper auto-detects from the first 30 s. `Some("de")`,
    /// `Some("en")`, etc. forces a single language (faster + slightly
    /// more accurate when the user knows what they speak).
    pub language: Option<String>,
}

/// Decode `audio_path` into a 16 kHz mono `Vec<f32>` whisper can ingest.
fn decode_to_pcm_f32_16k(audio_path: &Path) -> Result<Vec<f32>> {
    let file = std::fs::File::open(audio_path)
        .with_context(|| format!("opening {}", audio_path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    // Probe by extension first — we control the path (it's
    // `<msg_id>.<ext>` written by `handle_incoming_voice_v1`), so the
    // extension is trustworthy. symphonia falls back to byte sniffing
    // even if we get the hint wrong.
    let mut hint = Hint::new();
    if let Some(ext) = audio_path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .with_context(|| "symphonia probe failed")?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("no decodable audio track in {}", audio_path.display()))?;
    let track_id = track.id;
    let source_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| anyhow!("source has no declared sample rate"))?;
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .with_context(|| {
            // The most common failure here is opus-in-ogg: symphonia 0.5
            // parses the OGG container but ships no Opus codec. Surface
            // a recognisable error so the caller can audit-log it.
            format!(
                "symphonia has no decoder for codec {:?} (opus is unsupported in pure-Rust 0.5 — \
                 sender must use AAC/M4A or wait for a future symphonia release)",
                track.codec_params.codec
            )
        })?;

    let mut interleaved: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                // Stream-format reset — for a single voice clip we just
                // bail (whisper has no use for partial decode).
                break;
            }
            Err(e) => return Err(anyhow!("symphonia next_packet: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => append_buffer_as_f32(&buf, &mut interleaved),
            Err(SymphoniaError::DecodeError(e)) => {
                // Single-packet decode failures are recoverable — skip
                // the bad packet rather than aborting the whole clip.
                // (Use eprintln! rather than `tracing` directly so this
                // module doesn't take a top-level dep on the tracing
                // crate; `whisper-rs` already pulls it in transitively
                // for the `tracing_backend` feature, but our wrapper
                // shouldn't depend on that being enabled.)
                eprintln!("stt: skipping bad packet: {e}");
                continue;
            }
            Err(e) => return Err(anyhow!("symphonia decode: {e}")),
        }
    }

    if interleaved.is_empty() {
        return Err(anyhow!("decoded zero samples from {}", audio_path.display()));
    }

    // Mix to mono.
    let mono: Vec<f32> = if channels <= 1 {
        interleaved
    } else {
        interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / (channels as f32))
            .collect()
    };

    // Resample to 16 kHz (linear interpolation — fine for speech). When
    // the source is already 16 kHz this is a zero-cost passthrough.
    let resampled = if source_rate == WHISPER_SAMPLE_RATE {
        mono
    } else {
        linear_resample_to_16k(&mono, source_rate)
    };

    Ok(resampled)
}

/// Append a `AudioBufferRef` (whatever sample format symphonia hands us)
/// to `out` as interleaved f32 in `[-1.0, 1.0]`.
///
/// The match below repeats the same nested-loop shape per arm rather
/// than going through a generic helper because the integer→f32
/// conversions need bespoke offsets/scales (signed-vs-unsigned ranges,
/// 24-bit `inner()` extraction). The repetition keeps each branch
/// independently auditable.
fn append_buffer_as_f32(buf: &AudioBufferRef<'_>, out: &mut Vec<f32>) {
    use symphonia::core::audio::AudioBufferRef::*;

    macro_rules! push_signed_int {
        ($buf:expr, $scale:expr) => {{
            let b = $buf;
            let chans = b.spec().channels.count();
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..chans {
                    out.push(b.chan(c)[f] as f32 / ($scale as f32));
                }
            }
        }};
    }
    macro_rules! push_unsigned_int {
        ($buf:expr, $offset:expr, $scale:expr) => {{
            let b = $buf;
            let chans = b.spec().channels.count();
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..chans {
                    out.push((b.chan(c)[f] as f32 - $offset) / $scale);
                }
            }
        }};
    }

    match buf {
        F32(b) => {
            let chans = b.spec().channels.count();
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..chans {
                    out.push(b.chan(c)[f]);
                }
            }
        }
        F64(b) => {
            let chans = b.spec().channels.count();
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..chans {
                    out.push(b.chan(c)[f] as f32);
                }
            }
        }
        S8(b) => push_signed_int!(b, i8::MAX),
        S16(b) => push_signed_int!(b, i16::MAX),
        S24(b) => {
            let chans = b.spec().channels.count();
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..chans {
                    out.push(b.chan(c)[f].inner() as f32 / 8_388_607.0);
                }
            }
        }
        S32(b) => push_signed_int!(b, i32::MAX),
        U8(b) => push_unsigned_int!(b, 128.0_f32, 128.0_f32),
        U16(b) => push_unsigned_int!(b, 32_768.0_f32, 32_768.0_f32),
        U24(b) => {
            let chans = b.spec().channels.count();
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..chans {
                    out.push((b.chan(c)[f].inner() as f32 - 8_388_608.0) / 8_388_608.0);
                }
            }
        }
        U32(b) => {
            let chans = b.spec().channels.count();
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..chans {
                    out.push(
                        (b.chan(c)[f] as f32 - 2_147_483_648.0) / 2_147_483_648.0,
                    );
                }
            }
        }
    }
}

/// Linear-interpolation resampler. Adequate for whisper's tolerance —
/// the model itself does much more aggressive frequency-domain work and
/// shrugs at minor anti-aliasing artefacts. Replace with `rubato` if a
/// future PR shows measurable WER improvement.
fn linear_resample_to_16k(input: &[f32], source_rate: u32) -> Vec<f32> {
    if source_rate == WHISPER_SAMPLE_RATE {
        return input.to_vec();
    }
    let ratio = WHISPER_SAMPLE_RATE as f64 / source_rate as f64;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_idx = i as f64 / ratio;
        let lo = src_idx.floor() as usize;
        let hi = (lo + 1).min(input.len().saturating_sub(1));
        let frac = (src_idx - lo as f64) as f32;
        let s = input.get(lo).copied().unwrap_or(0.0);
        let e = input.get(hi).copied().unwrap_or(s);
        out.push(s + (e - s) * frac);
    }
    out
}

/// Run whisper.cpp on the audio at `audio_path` and return the
/// concatenated transcription. Blocking — call from a tokio
/// `spawn_blocking` if the caller is on an async runtime.
pub fn transcribe(cfg: &WhisperConfig, audio_path: &Path) -> Result<String> {
    if !cfg.enabled {
        return Err(anyhow!("STT disabled in config"));
    }
    if !cfg.model_path.is_file() {
        return Err(anyhow!(
            "whisper model not found at {} — download via Settings → AI Bridge → STT",
            cfg.model_path.display()
        ));
    }

    let pcm = decode_to_pcm_f32_16k(audio_path)?;

    // Hazard: previously called `WhisperContext::new_with_params` per
    // transcription (100-300 ms mmap + GGML init). Hold a cached Arc'd
    // context keyed by model path; release the mutex before inference so
    // parallel transcribes against the same model can run concurrently.
    // Evict + reload only when the configured model_path changes.
    let ctx: Arc<WhisperContext> = {
        let mut cache = whisper_cache()
            .lock()
            .map_err(|_| anyhow!("whisper cache mutex poisoned"))?;
        if cache
            .as_ref()
            .map(|c| c.model_path != cfg.model_path)
            .unwrap_or(true)
        {
            let ctx_params = WhisperContextParameters::default();
            let new_ctx = WhisperContext::new_with_params(
                cfg.model_path.to_string_lossy().as_ref(),
                ctx_params,
            )
            .with_context(|| format!("loading whisper model {}", cfg.model_path.display()))?;
            *cache = Some(CachedWhisper {
                model_path: cfg.model_path.clone(),
                ctx: Arc::new(new_ctx),
            });
        }
        Arc::clone(&cache.as_ref().expect("just populated above").ctx)
    };
    let mut state = ctx
        .create_state()
        .with_context(|| "creating whisper state")?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    if let Some(lang) = cfg.language.as_deref() {
        params.set_language(Some(lang));
    }
    // Multi-threaded inference. whisper.cpp picks a sensible default,
    // but on modern desktops 4 threads is the WER/latency sweet spot.
    params.set_n_threads(num_cpus_or_default(4) as i32);

    state
        .full(params, &pcm)
        .with_context(|| "whisper full() inference")?;

    // 0.16 API: `full_n_segments()` returns `c_int` directly (not a
    // Result), and segments come out via `state.get_segment(i) ->
    // Option<WhisperSegment>` whose `to_str_lossy()` yields the text.
    let n = state.full_n_segments();
    let mut out = String::new();
    for i in 0..n {
        let seg = match state.get_segment(i) {
            Some(s) => s,
            None => continue,
        };
        let text_cow = seg
            .to_str_lossy()
            .with_context(|| format!("whisper segment {i} text"))?;
        let text = text_cow.as_ref();
        if !out.is_empty() && !text.starts_with(' ') {
            out.push(' ');
        }
        out.push_str(text.trim());
    }

    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        return Err(anyhow!("whisper produced empty transcription"));
    }
    Ok(trimmed)
}

/// Best-effort thread-count detection. We don't pull `num_cpus` for one
/// helper — `available_parallelism` covers the same ground in std and
/// only fails on locked-down kernels (rare; fall back to `default`).
fn num_cpus_or_default(default: usize) -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(default)
}
