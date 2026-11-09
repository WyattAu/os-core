#!/usr/bin/env bash
# REQ-OS-004 spike runner: doorbell-driven submission economics on Linux.
# Builds the excluded spikes/io-spike crate in release mode and prints TSV:
#   mode  ops  ns_per_op  ops_per_sec
# Requires kernel io_uring enabled (/proc/sys/kernel/io_uring_disabled == 0).
set -euo pipefail
cd "$(dirname "$0")/.."
cargo run --release --quiet --manifest-path spikes/io-spike/Cargo.toml -- "$@"
