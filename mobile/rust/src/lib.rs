//! Mobile-side flutter_rust_bridge wrapper around `phantomchat_core`.
//!
//! This crate exists because the v3 desktop release added wire-format
//! prefixes (`MLS-WLC2`, `MLS-APP1`, `FILE1:01`, `RCPT-1:`, `TYPN-1:`) and
//! sealed-sender attribution that the previous mobile bindings — generated
//! against `core/src/api.rs` — never exposed. The wave-7B brief disallows
//! modifying anything outside `mobile/`, so the v3 surface is bridged here
//! instead of being added back to the core crate.
//!
//! The Dart side imports `lib/src/rust/api.dart` (auto-generated from this
//! crate). Existing v2 entry points (`start_network_node`,
//! `scan_incoming_envelope`, `send_secure_message`, …) are re-exported so
//! the existing Flutter screens keep compiling.

pub mod api;

mod frb_generated; // AUTO-INJECTED by `flutter_rust_bridge_codegen generate`.
