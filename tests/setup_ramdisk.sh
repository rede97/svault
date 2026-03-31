#!/usr/bin/env bash
# setup_ramdisk.sh — Create and maintain the svault test RAMDisk.
#
# Usage:
#   bash tests/setup_ramdisk.sh            # mount (idempotent)
#   bash tests/setup_ramdisk.sh --clean    # unmount + remount (fresh)
#   bash tests/setup_ramdisk.sh --umount   # unmount only
#
# The RAMDisk lives at /tmp/svault-ramdisk.
# run_tests.py will reuse it if already mounted.

set -euo pipefail

RAMDISK="/tmp/svault-ramdisk"
SIZE="128m"
ACTION="mount"

for arg in "$@"; do
  case $arg in
    --clean)  ACTION="clean" ;;
    --umount) ACTION="umount" ;;
  esac
done

is_mounted() { mountpoint -q "$RAMDISK" 2>/dev/null; }

do_umount() {
  if is_mounted; then
    echo "Unmounting $RAMDISK"
    sudo umount "$RAMDISK" 2>/dev/null || umount "$RAMDISK" 2>/dev/null || true
    echo "Unmounted."
  else
    echo "$RAMDISK is not mounted."
  fi
}

do_mount() {
  mkdir -p "$RAMDISK"
  if is_mounted; then
    echo "$RAMDISK already mounted — nothing to do."
    return
  fi
  if ! mount -t tmpfs -o "size=$SIZE" tmpfs "$RAMDISK" 2>/dev/null; then
    sudo mount -t tmpfs -o "size=$SIZE" tmpfs "$RAMDISK"
  fi
  sudo chown "$(id -u):$(id -g)" "$RAMDISK" 2>/dev/null || true
  echo "RAMDisk mounted at $RAMDISK ($SIZE)"
}

case $ACTION in
  mount)  do_mount ;;
  umount) do_umount ;;
  clean)
    do_umount
    do_mount
    echo "RAMDisk cleaned and remounted."
    ;;
esac
