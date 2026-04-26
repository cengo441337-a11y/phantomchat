// Fuzz TYPN-1: typing-indicator parser. Same dispatch story as RCPT-1 —
// today it's the Payload deserialiser; PR #3 introduces a dedicated
// `phantomchat_core::typing::parse_v1`. Inner call swap-only.
#![no_main]
use libfuzzer_sys::fuzz_target;
use phantomchat_core::Payload;

fuzz_target!(|data: &[u8]| {
    let _ = Payload::from_bytes(data);
});
