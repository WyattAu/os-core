#!/usr/bin/env bash
# Kani model-check leg (ADR-0014 rung C+): cursor algebra proven over
# the actual Rust (omni-ring's SPSC, symbolically incl. the wrap seam).
# LOCAL leg only — CI runners don't ship kani (like repro/bench).
set -euo pipefail
command -v cargo-kani >/dev/null 2>&1 || {
    echo "kani missing (cargo install --locked kani-verifier)" >&2
    exit 2
}
cd "$(dirname "$0")/../crates/omni-ring"
cargo kani
echo "kani: OK"
