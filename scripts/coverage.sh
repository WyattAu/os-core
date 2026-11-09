#!/usr/bin/env bash
# llvm-cov gate — tier a threshold (>=90%). Override: ./scripts/coverage.sh 80
set -euo pipefail
cd "$(dirname "$0")/.."
THRESHOLD="${1:-90}"
# cargo-llvm-cov renamed the threshold flag across versions (0.9.x uses
# --fail-under-lines); pick whichever this build understands.
if cargo llvm-cov --help 2>&1 | grep -q -- '--fail-under '; then
    exec cargo llvm-cov --workspace --all-features --summary-only --fail-under "$THRESHOLD"
else
    exec cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines "$THRESHOLD"
fi
