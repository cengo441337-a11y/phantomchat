// Fuzz `nostr_extract_event_payload` (Wave 7B2). The function lives in
// the relays crate and pulls the inner ciphertext bytes out of a Nostr
// `EVENT` JSON-RPC message. It's the very first parser on every relay-
// inbound packet — must be panic-free for arbitrary bytes.
//
// Until the named function lands, the closest equivalent surface is the
// Envelope::from_bytes path that the extracted payload feeds into. Swap
// the inner call when the dedicated extractor ships.
#![no_main]
use libfuzzer_sys::fuzz_target;
use phantomchat_core::Envelope;

fuzz_target!(|data: &[u8]| {
    // Best-effort surrogate: feed bytes that *would* be the extracted
    // event payload directly into the envelope parser.
    let _ = Envelope::from_bytes(data);
});
