#!/usr/bin/env bash
# setup_ramdisk.sh — Create and maintain the svault test RAMDisk.
#
# Usage:
#   ./setup_ramdisk.sh                      # mount (idempotent, default 128m)
#   ./setup_ramdisk.sh --size 256m          # mount with custom size
#   ./setup_ramdisk.sh --clean              # unmount + remount (fresh)
#   ./setup_ramdisk.sh --clean --size 512m  # unmount + remount with custom size
#   ./setup_ramdisk.sh --umount             # unmount only
#
# The RAMDisk lives at /tmp/svault-ramdisk.
# run.sh will reuse it if already mounted.

set -euo pipefail

RAMDISK="/tmp/svault-ramdisk"
SIZE="128m"
ACTION="mount"

while [[ $# -gt 0 ]]; do
  case $1 in
    --clean)  ACTION="clean"; shift ;;
    --umount) ACTION="umount"; shift ;;
    --size)
      if [[ $# -lt 2 ]]; then
        echo "Error: --size requires an argument (e.g., 256m, 1g)" >&2
        exit 1
      fi
      SIZE="$2"
      shift 2
      ;;
    *)
      echo "Unknown option: $1" >&2
      echo "Usage: $0 [--size SIZE] [--clean|--umount]" >&2
      exit 1
      ;;
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
  
  # Remove symlink if exists
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
  LINK_NAME="$PROJECT_ROOT/.ramdisk"
  
  if [[ -L "$LINK_NAME" ]]; then
    rm "$LINK_NAME"
    echo "Symlink removed: .ramdisk"
  fi
}

ensure_symlink() {
  # Create or update symlink in project root for easy access
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
  LINK_NAME="$PROJECT_ROOT/.ramdisk"
  
  if [[ -L "$LINK_NAME" ]]; then
    # Check if symlink points to correct target
    CURRENT_TARGET="$(readlink "$LINK_NAME")"
    if [[ "$CURRENT_TARGET" != "$RAMDISK" ]]; then
      rm "$LINK_NAME"
      ln -sf "$RAMDISK" "$LINK_NAME"
      echo "Symlink updated: .ramdisk -> $RAMDISK"
    else
      echo "Symlink already exists: .ramdisk -> $RAMDISK"
    fi
  else
    ln -sf "$RAMDISK" "$LINK_NAME"
    echo "Symlink created: .ramdisk -> $RAMDISK"
  fi
}

do_mount() {
  mkdir -p "$RAMDISK"
  if is_mounted; then
    echo "$RAMDISK already mounted."
    ensure_symlink
    return
  fi
  if ! mount -t tmpfs -o "size=$SIZE" tmpfs "$RAMDISK" 2>/dev/null; then
    sudo mount -t tmpfs -o "size=$SIZE" tmpfs "$RAMDISK"
  fi
  sudo chown "$(id -u):$(id -g)" "$RAMDISK" 2>/dev/null || true
  
  ensure_symlink
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
