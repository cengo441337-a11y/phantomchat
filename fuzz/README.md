# PhantomChat fuzz harnesses

This directory holds [`cargo-fuzz`](https://rust-fuzz.github.io/book/) targets
covering every wire-format parser exposed to attacker-controlled bytes.
A single panic in any of them is a potential remote DoS vector — and, on a
panic in unsafe-deserialisation territory, a potential RCE primitive.

## Targets

| Target                  | What it fuzzes                                               | Wire format         |
|-------------------------|--------------------------------------------------------------|---------------------|
| `envelope_parse`        | `phantomchat_core::Envelope::from_bytes`                     | sealed envelope     |
| `mls_welcome_v2`        | `PhantomMlsMember::join_via_welcome`                         | MLS-WLC2            |
| `mls_app_v1`            | `PhantomMlsGroup::decrypt`                                   | MLS-APP1            |
| `file_v1`               | `file_transfer::peek_header`                                 | FILE1:01            |
| `receipt_v1`            | `Payload::from_bytes` (PR #3 swaps to `receipts::parse_v1`)  | RCPT-1:             |
| `typing_v1`             | `Payload::from_bytes` (PR #3 swaps to `typing::parse_v1`)    | TYPN-1:             |
| `reply_v1`              | `Payload::from_bytes` (PR #3 swaps to `replies::parse_v1`)   | REPL-1:             |
| `reaction_v1`           | `Payload::from_bytes` (PR #3 swaps to `reactions::parse_v1`) | RACT-1:             |
| `disappearing_v1`       | `Envelope::from_bytes` (PR #3 swaps to disa-flag parser)     | DISA-1:             |
| `phantomaddress_parse`  | `PhantomAddress::parse`                                      | phantom: / phantomx:|
| `nostr_event_extract`   | nostr `EVENT`-frame extractor (Wave 7B2 surrogate)           | nostr-relay         |

## Run a single target locally

```bash
# Smoke run (5 minutes)
cargo +nightly fuzz run envelope_parse -- -max_total_time=300

# Full overnight run (8h, multi-core, 16 threads)
cargo +nightly fuzz run envelope_parse -- -max_total_time=28800 -workers=16 -jobs=16

# Replay a saved corpus crash
cargo +nightly fuzz run envelope_parse -- artifacts/envelope_parse/crash-<hash>
```

## CI integration

`.github/workflows/ci.yml` includes a `fuzz-smoke` job that runs each target
for 30 seconds on every push. That's enough to catch obvious regressions
without burning a CI hour. Deeper, longer fuzzing should be done out-of-band
on a dedicated runner — the corpus + crash artifacts under `fuzz/corpus/`
and `fuzz/artifacts/` should be checked in by hand for any reproducer.

## Adding a new target

1. Add a file to `fuzz_targets/<name>.rs` with the `fuzz_target!` macro.
2. Add a matching `[[bin]]` block to `fuzz/Cargo.toml`.
3. Add a row to the table above.
4. Add a corpus seed under `fuzz/corpus/<name>/seed-01` (any valid example
   of the wire format you're fuzzing — accelerates coverage by ~10x).

## Toolchain

`cargo-fuzz` requires nightly Rust. The CI job installs it via
`rustup toolchain install nightly`. Locally:

```bash
rustup toolchain install nightly
cargo install cargo-fuzz --locked
```
