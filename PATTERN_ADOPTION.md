# Pattern adoption

| Pattern | Status | Evidence |
|---|---|---|
| T1 | adopted | crates/omni-core/src/text.rs:37 (PubId::parse returns Result<Self, ParseError>) |
| T13 | adopted | crates/omni-core/src/lib.rs:1 (L0 library crates are the core), crates/omni-service/src/main.rs:1 (service binary is the shell) |
| T20 | adopted | crates/omni-ring/src/spsc.rs:34 (SpscRing: single-writer, lock-free ring) |
| T40 | adopted | crates/omni-sbe/src/lib.rs:63 (zero-copy, bounds-checked view over an SBE frame) |
| T8 | partial | crates/omni-cap/src/lib.rs:42 (HardwareCapability carries base, bound, permissions, epoch) and crates/omni-cap/src/lib.rs:6 (no Clone or Copy, so capabilities cannot be duplicated); explicit attenuation paths are not yet exercised |
