#!/usr/bin/env bash
# REQ-OS-005 / ADR-0013: boot omniosk-kernel in QEMU and assert the
# verified-ring smoke over serial, with a clean isa-debug-exit status.
# NVMe markers are REQUIRED (full NVMe driver path: identify -> IO queues -> read).
set -euo pipefail
cd "$(dirname "$0")/../kernels/omniosk-kernel"

command -v qemu-system-x86_64 >/dev/null || {
    echo "qemu-system-x86_64 missing (host: pacman -S qemu; CI: apt qemu-system-x86)" >&2
    exit 2
}
rustup target list --installed | grep -q x86_64-unknown-none || {
    echo "x86_64-unknown-none target missing (rustup target add x86_64-unknown-none)" >&2
    exit 2
}

# Scratch NVMe drive: 16 MiB raw image with the data-round-trip magic in
# LBA 0 (the kernel reads sector 0 and compares). Idempotent.
mkdir -p tests
truncate -s 16M tests/nvme.img
printf 'OMNIOSK-NVME-OK!' | dd of=tests/nvme.img bs=1 seek=0 conv=notrunc status=none

cargo build --release

# The fleet may redirect the target dir globally; resolve the real path.
BIN="$(cargo metadata --format-version 1 --no-deps \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/x86_64-unknown-none/release/omniosk-kernel"

log="$(mktemp)"
trap 'rm -f "$log"' EXIT

set +e
timeout 60 qemu-system-x86_64 \
    -kernel "$BIN" \
    -drive file=tests/nvme.img,if=none,format=raw,id=nvmedrive \
    -device nvme,serial=OMNIOSK,drive=nvmedrive \
    -serial "file:$log" \
    -display none \
    -no-reboot \
    -m 128M \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04
status=$?
set -e

if [ "$status" -ne 127 ]; then
    echo "qemu-omniosk: FAIL (exit status $status; success is 127, reset is 0)" >&2
    cat "$log" >&2
    exit 1
fi
for marker in "OMNIOSK UP" "RING WRAP PASS" "RING SMOKE PASS"; do
    grep -q "$marker" "$log" || {
        echo "qemu-omniosk: FAIL (serial missing '$marker')" >&2
        cat "$log" >&2
        exit 1
    }
done
# NVMe markers are REQUIRED: the driver completes the full path (PCI
# scan -> controller init -> Identify -> Create IOCQ/IOSQ -> DMA read).
nvme_missing=0
for marker in "NVME IDENT PASS" "NVME READ PASS"; do
    grep -q "$marker" "$log" || {
        echo "qemu-omniosk: WARN (serial missing '$marker' — NVMe WIP)" >&2
        nvme_missing=1
    }
done
[ "$nvme_missing" -eq 1 ] && exit 1
echo "qemu-omniosk: OK"
