// Fuzz REPL-1: reply-thread parser (PR #3). Surrogate-wired to the
// Payload entry point until the dedicated parser ships.
#![no_main]
use libfuzzer_sys::fuzz_target;
use phantomchat_core::Payload;

fuzz_target!(|data: &[u8]| {
    let _ = Payload::from_bytes(data);
});
