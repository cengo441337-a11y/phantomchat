// Fuzz RCPT-1: read-receipt parser. The receipt wire format ships inside
// a Payload body, so the first parser an attacker-controlled byte stream
// hits is `Payload::from_bytes`. When PR #3 lands a dedicated
// `phantomchat_core::receipts::parse_v1` entry point, swap the inner call
// in this harness — the corpus stays valid.
#![no_main]
use libfuzzer_sys::fuzz_target;
use phantomchat_core::Payload;

fuzz_target!(|data: &[u8]| {
    let _ = Payload::from_bytes(data);
});
