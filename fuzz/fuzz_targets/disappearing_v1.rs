// Fuzz DISA-1: disappearing-message envelope-flag parser (PR #3).
// Surrogate-wired to the Envelope entry point — DISA-1 metadata rides in
// the envelope header rather than the payload, so Envelope::from_bytes is
// the touched surface.
#![no_main]
use libfuzzer_sys::fuzz_target;
use phantomchat_core::Envelope;

fuzz_target!(|data: &[u8]| {
    let _ = Envelope::from_bytes(data);
});
