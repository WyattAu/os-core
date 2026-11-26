#!/usr/bin/env bash
# Limine-path gate (ADR-0014 oracle policy): boots the deployed artifact
# through the REAL bootloader the server runs — Limine UEFI (multiboot2
# protocol) via OVMF — and asserts the full hardware-profile smoke over
# serial. QEMU `-kernel` boots via the PVH note and never exercises the
# multiboot2 header; a malformed header (e.g. mis-sized tags) only
# panics at the bootloader on the real machine. This gate closes that
# gap: the artifact must boot through Limine before it deploys.
#
# Asserts (serial, in order): the multiboot2 banner, NVMe hw
# read-consistency, CMOS markers written, and the mb2 halt line.
set -euo pipefail
cd "$(dirname "$0")/../kernels/omniosk-kernel"

command -v qemu-system-x86_64 >/dev/null || {
    echo "qemu-system-x86_64 missing" >&2
    exit 2
}
command -v mformat >/dev/null || {
    echo "mtools missing (mformat/mcopy required to build the boot image)" >&2
    exit 2
}
rustup target list --installed | grep -q x86_64-unknown-none || {
    echo "x86_64-unknown-none target missing" >&2
    exit 2
}

LIMINE_EFI="tests/limine_x64.efi"
[ -f "$LIMINE_EFI" ] || {
    echo "vendored Limine missing: $LIMINE_EFI (the server's bootloader)" >&2
    exit 2
}

# Locate OVMF firmware across distros.
OVMF=""
for p in /usr/share/edk2/x64/OVMF_CODE.4m.fd /usr/share/OVMF/OVMF_CODE.fd \
         /usr/share/ovmf/OVMF_CODE.fd /usr/share/qemu/OVMF_CODE.fd; do
    [ -f "$p" ] && OVMF="$p" && break
done
[ -n "$OVMF" ] || {
    echo "OVMF firmware not found (edk2-ovmf / ovmf package)" >&2
    exit 2
}
VARS_DIR="$(dirname "$OVMF")"
VARS_SRC="$VARS_DIR/OVMF_VARS.4m.fd"
[ -f "$VARS_SRC" ] || VARS_SRC="$VARS_DIR/OVMF_VARS.fd"

cargo build --release

TARGET_DIR="$(cargo metadata --format-version 1 --no-deps \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
BIN="$TARGET_DIR/x86_64-unknown-none/release/omniosk-kernel"

# The boot medium: FAT32 with the ESP layout the server has —
# limine.conf at the root, the kernel ELF, Limine as the fallback loader.
mkdir -p tests
rm -f tests/limine-boot.img tests/limine-vars.fd tests/nvme-hw.img
dd if=/dev/zero of=tests/limine-boot.img bs=1M count=64 status=none
mformat -F -i tests/limine-boot.img ::/
printf 'serial: yes\ntimeout: 5\ndefault_entry: 1\n\n/OMNIOSK\n  comment: omniosk-kernel hardware smoke (phase 2)\n  protocol: multiboot2\n  path: boot():/omniosk.elf\n' > tests/limine.conf
mcopy -i tests/limine-boot.img tests/limine.conf ::/limine.conf
mcopy -i tests/limine-boot.img "$BIN" ::/omniosk.elf
mmd -i tests/limine-boot.img ::/EFI
mmd -i tests/limine-boot.img ::/EFI/BOOT
mcopy -i tests/limine-boot.img "$LIMINE_EFI" ::/EFI/BOOT/BOOTX64.EFI
truncate -s 16M tests/nvme-hw.img
cp "$VARS_SRC" tests/limine-vars.fd

log="$(mktemp)"
trap 'rm -f "$log"' EXIT

# The hw profile (multiboot2 == boot_kind 2) runs the read-only smoke:
# NVMe read-consistency, CMOS markers, no write test, then the mb2 halt.
set +e
timeout 120 qemu-system-x86_64 \
    -m 512 \
    -drive if=pflash,format=raw,file="$OVMF",readonly=on \
    -drive if=pflash,format=raw,file=tests/limine-vars.fd \
    -drive file=tests/limine-boot.img,if=none,format=raw,id=bootdrv \
    -device nvme,serial=OMNIOSK,drive=bootdrv \
    -serial file:"$log" \
    -display none -no-reboot
status=$?
set -e

fail() {
    echo "qemu-limine: FAIL ($1; qemu status $status)" >&2
    cat "$log" >&2
    exit 1
}

# Limine's serial mirror emits characters wrapped in ANSI cursor-position
# sequences, so contiguous greps fail on the raw stream. Strip the
# escapes first (the assertions below match the plain text).
python3 - "$log" <<'PYEOF'
import re, sys
raw = open(sys.argv[1], 'rb').read().decode(errors='replace')
text = re.sub(r'\x1b\[[0-9;]*[a-zA-Z]', '', raw)
text = re.sub(r'[^\x20-\x7e\n]', '', text)
open(sys.argv[1] + '.clean', 'w').write(text)
PYEOF
log="$log.clean"

grep -aq "Limine" "$log" || fail "Limine never ran"
grep -aq "PANIC" "$log" && fail "Limine panicked"
grep -aq "OMNIOSK UP (multiboot2 -> long mode)" "$log" \
    || fail "the multiboot2 handoff never reached the kernel"
grep -aq "NVME READ CONSISTENT (hw)" "$log" \
    || fail "hw read-consistency smoke failed"
grep -aq "CMOS: markers written" "$log" || fail "CMOS markers not written"
grep -aq "RESET (multiboot2 smoke done" "$log" \
    || fail "the kernel never reached its reset line"

echo "qemu-limine: OK"
