// Fuzz `Envelope::from_bytes` — the universal wire-format entry point that
// every relay-delivered ciphertext flows through. A panic here = potential
// remote DoS or memory-corruption vector if/when the codebase ever gains
// `unsafe` deserialisation. Keep this harness clean of any logic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use phantomchat_core::Envelope;

fuzz_target!(|data: &[u8]| {
    // from_bytes returns Option, never panics-in-spec; we're confirming
    // that property holds across arbitrary inputs.
    let _ = Envelope::from_bytes(data);
});
