# S1 public contract

This directory is the language-neutral source for WP01's public wire contract.
It is not a provider protocol and intentionally has no native thread/turn
requirement.

Authority order:

1. schema.json freezes field names, required/nullability rules, state/error
   vocabulary, version 1, and published limits from C02/C04/C05/C07/C08.
2. fixtures/manifest.json fixes shared positive and negative byte fixtures.
3. ../../src-tauri/src/public_core/ is the Rust model and strict codec that
   implements that schema.

Canonical JSON uses UTF-8, no insignificant whitespace, object keys sorted by
their UTF-8 bytes, array order preserved, and integer JSON numbers only.
sequence, generation, revision, epochs, and tick counters are instead
canonical unsigned decimal strings in the inclusive range
0..18446744073709551615. Leading zeroes, signs, overflow, floats, exponents,
negative zero, duplicate keys, invalid UTF-8, missing required fields, and
unknown schema majors are rejected before any state or authority change.

The schema deliberately leaves event payload, native receipts, and permission
ceilings as JSON data owned by their action/kind-specific validators. Their
presence here does not grant authority or import a provider-native shape.

Until the Controller integrates public_core into the desktop crate's shared
module registry, this directory is also a no-hotspot compilation harness:

    cargo +1.89.0 test --locked --manifest-path apps/desktop/contracts/s1/Cargo.toml

Controller integration still needs one shared-hotspot patch to register
src/public_core/mod.rs; this task does not modify lib.rs, shared/mod.rs, or the
desktop manifest.

