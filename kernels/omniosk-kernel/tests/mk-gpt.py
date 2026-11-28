#!/usr/bin/env python3
"""Builds the gate's NVMe disk: a minimal valid GPT whose p1 is the
ESP (the mformatted limine-boot medium, embedded at LBA 2048) and
whose p2 is the omniosk-scratch partition for the write smoke.
Mirrors the real box's layout (ESP + scratch on one GPT disk)."""
import struct
import sys
import uuid
import zlib

nvme_img, esp_img = sys.argv[1], sys.argv[2]
esp = bytearray(open(esp_img, 'rb').read())
esp_sectors = (len(esp) + 511) // 512

esp_start = 2048
scratch_start = esp_start + esp_sectors
total_sectors = scratch_start + 2048 + 34  # scratch + backup GPT tail
scratch_end = scratch_start + 2047

img = bytearray(total_sectors * 512)

# Protective MBR at LBA 0 (UEFI requires a valid MBR signature 0x55AA
# with a 0xEE protective entry covering the disk).
mbr = bytearray(512)
mbr[446] = 0x00  # boot flag: not bootable
mbr[447:450] = b'\xff\xff\xff'  # CHS start (unused)
mbr[450] = 0xEE  # type: GPT protective
mbr[451:454] = b'\xff\xff\xff'  # CHS end (unused)
struct.pack_into('<I', mbr, 454, 1)  # start LBA (the GPT itself)
struct.pack_into('<I', mbr, 458, min(total_sectors - 1, 0xFFFFFFFF))
struct.pack_into('<H', mbr, 510, 0xAA55)
img[0:512] = mbr

TYPE_EFI = uuid.UUID('c12a7328-f81f-11d2-ba4b-00a0c93ec93b').bytes_le
TYPE_LINUX_FS = uuid.UUID('0fc63daf-8483-4772-8e79-3d69d8477de4').bytes_le
UGUID_ESP = uuid.uuid5(uuid.NAMESPACE_URL, 'omniosk-esp-qemu').bytes_le
UGUID_SCRATCH = uuid.uuid5(uuid.NAMESPACE_URL, 'omniosk-scratch-qemu').bytes_le

arr = bytearray(512 * 8)  # 8 sectors = 4096 B = 32 entries x 128 B

def entry(slot, type_guid, first, last, name, uguid):
    off = slot * 128
    e = bytearray(128)
    e[0:16] = type_guid
    e[16:32] = uguid
    struct.pack_into('<QQ', e, 32, first, last)
    nm = name.encode('utf-16-le')
    e[56:56 + len(nm)] = nm
    arr[off:off + 128] = e

entry(0, TYPE_EFI, esp_start, esp_start + esp_sectors - 1, 'ESP', UGUID_ESP)
entry(1, TYPE_LINUX_FS, scratch_start, scratch_end, 'omniosk-scratch', UGUID_SCRATCH)

crc_arr = zlib.crc32(bytes(arr[:512 * 8])) & 0xffffffff

DISK_GUID = uuid.uuid5(uuid.NAMESPACE_URL, 'omniosk-scratch-disk').bytes_le

def header(current_lba, backup_lba, arr_lba):
    h = bytearray(512)
    h[0:8] = b'EFI PART'
    h[56:72] = DISK_GUID
    struct.pack_into('<II', h, 8, 0x00010000, 92)
    struct.pack_into('<Q', h, 24, current_lba)
    struct.pack_into('<Q', h, 32, backup_lba)
    struct.pack_into('<Q', h, 40, 34)
    struct.pack_into('<Q', h, 48, total_sectors - 34)
    struct.pack_into('<Q', h, 72, arr_lba)
    struct.pack_into('<II', h, 80, 32, 128)
    struct.pack_into('<I', h, 88, crc_arr)
    return h

h1 = header(1, total_sectors - 1, 2)
struct.pack_into('<I', h1, 16, 0)
struct.pack_into('<I', h1, 16, zlib.crc32(bytes(h1[:92])) & 0xffffffff)
img[512:1024] = h1
img[1024:1024 + 512 * 8] = bytes(arr[:512 * 8])
img[esp_start * 512:(esp_start + esp_sectors) * 512] = bytes(esp)

arr_bak_lba = total_sectors - 33
h2 = header(total_sectors - 1, 1, arr_bak_lba)
struct.pack_into('<I', h2, 16, 0)
struct.pack_into('<I', h2, 16, zlib.crc32(bytes(h2[:92])) & 0xffffffff)
img[arr_bak_lba * 512:arr_bak_lba * 512 + 512 * 8] = bytes(arr[:512 * 8])
img[(total_sectors - 1) * 512:total_sectors * 512] = h2

open(nvme_img, 'wb').write(bytes(img))
print(f"GPT: ESP [{esp_start}..{esp_start + esp_sectors - 1}] "
      f"({esp_sectors} sectors), scratch [{scratch_start}..{scratch_end}], "
      f"disk {total_sectors} sectors")
