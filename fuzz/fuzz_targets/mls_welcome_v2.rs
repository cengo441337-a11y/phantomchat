// Fuzz the MLS-WLC2 wrapper-parser path. Today the public-API entry point
// for Welcome processing is `PhantomMlsMember::join_via_welcome`, which
// internally TLS-deserialises a Welcome message before handing it to
// openmls. The wrapper-parse step is the attacker-reachable surface (the
// rest is openmls's own attack surface, separately fuzzed upstream).
//
// We attempt a join with arbitrary bytes — a fresh PhantomMlsMember per
// invocation isolates state. Errors are expected and discarded; the only
// failure mode that matters is a panic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use phantomchat_core::mls::PhantomMlsMember;

fuzz_target!(|data: &[u8]| {
    // Skip vacuous inputs to keep the per-iteration cost bounded.
    if data.len() < 4 {
        return;
    }
    let Ok(mut member) = PhantomMlsMember::new(b"fuzz-identity".to_vec()) else {
        return;
    };
    let _ = member.join_via_welcome(data);
});
