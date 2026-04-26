// Fuzz `PhantomAddress::parse` — the string-parser for the on-wire
// stealth-address format ("phantom:<view_hex>:<spend_hex>",
// "phantomx:<...>" for hybrids). Public-facing input: pasted from QR
// scans, copy/paste, deeplinks. Must never panic regardless of input.
#![no_main]
use libfuzzer_sys::fuzz_target;
use phantomchat_core::PhantomAddress;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = PhantomAddress::parse(s);
    }
});
