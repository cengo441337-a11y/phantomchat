// Fuzz FILE1:01 manifest parser. The public read-path entry point is
// `phantomchat_core::file_transfer::peek_header`, which deserialises the
// archive header without needing a group key — it's the first thing a
// malicious peer's archive bytes touch.
#![no_main]
use libfuzzer_sys::fuzz_target;
use phantomchat_core::file_transfer::peek_header;

fuzz_target!(|data: &[u8]| {
    let _ = peek_header(data);
});
