#!/usr/bin/env bash
# T4b gate (docs/OTA-UPDATE.md / ADR-0014 oracle policy): the OTA loader
# end-to-end. The kernel ELF is staged on the NVMe scratch (header LBA
# 34, payload LBA 35+) with its banner string patched — the update-loaded
# kernel must print the PATCHED banner, proving the staged image (not the
# running one) executed through the hand-encoded trampoline.
#
# Boot sequence asserted:
#   boot 1 (PVH, kind 1): full smoke -> OTA: staged update found ->
#     OTA: ELF valid, jumping -> trampoline (32-bit compat drop)
#   boot 2 (update, kind 3): PATCHED banner -> hw-profile read-only
#     smoke -> CMOS markers -> 8042 reset (no exit device on the update path;
#
# The exit code cannot be used (kind 3 halts like multiboot2), so the
# gate reads the serial log, exactly as the multiboot2 path is gated.
set -euo pipefail
cd "$(dirname "$0")/../kernels/omniosk-kernel"

command -v qemu-system-x86_64 >/dev/null || {
    echo "qemu-system-x86_64 missing" >&2
    exit 2
}
rustup target list --installed | grep -q x86_64-unknown-none || {
    echo "x86_64-unknown-none target missing" >&2
    exit 2
}

cargo build --release

# The fleet may redirect the target dir globally; resolve the real path.
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
BIN="$TARGET_DIR/x86_64-unknown-none/release/omniosk-kernel"

# Stage the update: LBA 0 the data-round-trip magic, LBA 34 the header
# (OMUP magic u32 + size u32, little-endian), LBA 35+ the payload —
# the same ELF with the update-boot banner replaced by a proof string
# of identical length (byte-for-byte, same-length .rodata swap).
mkdir -p tests
python3 - "$BIN" <<'PYEOF'
import struct, sys

src = open(sys.argv[1], "rb").read()
orig = b"update-loaded -> long mode"
proof = b"UPDATE-LOADED: OTA PROOF 2"
assert len(orig) == len(proof), "proof string must match length"
assert src.count(orig) == 1, f"banner literal not unique: {src.count(orig)}"
payload = src.replace(orig, proof)

img = bytearray(16 * 1024 * 1024)
magic = b"OMNIOSK-NVME-OK!"
img[0 : len(magic)] = magic

header_lba, data_lba = 34, 35
assert len(payload) <= 512 * 1024, "payload exceeds update_channel::MAX_SIZE"
hdr = struct.pack("<II", 0x5055_4D4F, len(payload))  # "OMUP" LE + size
img[header_lba * 512 : header_lba * 512 + 8] = hdr
img[data_lba * 512 : data_lba * 512 + len(payload)] = payload

open("tests/nvme-ota.img", "wb").write(img)
print(f"staged: header LBA {header_lba}, payload {len(payload)} bytes "
      f"at LBA {data_lba} ({(len(payload) + 511) // 512} LBAs)")
PYEOF

log="$(mktemp)"
trap 'rm -f "$log"' EXIT

# The trampoline copies the payload over the running image and jumps;
# boot 2 halts on the update path (no isa-debug-exit exit on kind 3),
# so QEMU runs until the timeout and the gate reads the serial log.
set +e
timeout 90 qemu-system-x86_64 \
    -kernel "$BIN" \
    -drive file=tests/nvme-ota.img,if=none,format=raw,id=nvmedrive \
    -device nvme,serial=OMNIOSK,drive=nvmedrive \
    -serial "file:$log" \
    -display none \
    -no-reboot \
    -m 128M \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04
status=$?
set -e

fail() {
    echo "qemu-ota: FAIL ($1; qemu status $status)" >&2
    cat "$log" >&2
    exit 1
}

# The kind-3 boot ends in an intentional 8042 reset: with -no-reboot
# QEMU exits (status 0) the moment the guest requests it. Accept that
# exit, but ONLY alongside the verdict line in the serial log — a
# triple fault would exit 0 with a log that never reaches the reset.
if [ "$status" -ne 124 ]; then
    grep -q "RESET (update boot done" "$log" \
        || fail "qemu exited early without the reset verdict (triple fault?)"
fi

grep -c "OTA: staged update found" "$log" | grep -q '^1$' \
    || fail "staged-update detection must fire exactly once (re-apply loop)"
grep -q "OTA: ELF valid, jumping" "$log" || fail "loader never jumped"
grep -q "OMNIOSK UP (UPDATE-LOADED: OTA PROOF 2)" "$log" \
    || fail "patched payload banner absent — the staged image did not run"

# Order: the OTA lines precede the update boot's banner.
python3 - "$log" <<'PYEOF'
import sys
log = open(sys.argv[1]).read()
pairs = [
    ("OTA: ELF valid, jumping", "OMNIOSK UP (UPDATE-LOADED: OTA PROOF 2"),
    ("OMNIOSK UP (UPDATE-LOADED: OTA PROOF 2", "NVME READ PASS"),
    ("OMNIOSK UP (UPDATE-LOADED: OTA PROOF 2", "CMOS: markers written"),
]
for a, b in pairs:
    assert log.index(a) < log.index(b), f"order violated: {a!r} !< {b!r}"
print("order OK")
PYEOF

grep -q "RESET (update boot done" "$log" \
    || fail "update boot never reached its reset line"

echo "qemu-ota: OK"
