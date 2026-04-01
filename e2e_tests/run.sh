#!/bin/bash
# Run E2E tests with proper environment isolation
#
# Usage:
#   ./run.sh                           # Run all tests (debug build)
#   ./run.sh --release                 # Run with release build
#   ./run.sh --ramdisk-size 512m       # Use 512MB RAMDisk
#   ./run.sh --ramdisk-size 1g --cleanup  # Use 1GB RAMDisk and cleanup after
#   ./run.sh -v -k test_import         # Verbose, only matching tests

set -e

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Clean environment to avoid ROS conflicts
unset PYTHONPATH
unset PYTEST_PLUGINS

# Use venv python directly
PYTHON="$SCRIPT_DIR/.venv/bin/python"
PYTEST="$SCRIPT_DIR/.venv/bin/pytest"

# Parse options - pytest handles --ramdisk-* and --cleanup via conftest.py
PYTEST_ARGS=()
RELEASE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --release)
            RELEASE=true
            PYTEST_ARGS+=("$1")
            shift
            ;;
        --ramdisk-size|--ramdisk-path)
            PYTEST_ARGS+=("$1" "$2")
            shift 2
            ;;
        --cleanup)
            PYTEST_ARGS+=("$1")
            shift
            ;;
        *)
            PYTEST_ARGS+=("$1")
            shift
            ;;
    esac
done

# Ensure binary is built
if [ "$RELEASE" = true ]; then
    BINARY="$SCRIPT_DIR/../target/release/svault"
    BUILD_ARGS="--release -p svault-cli -q"
else
    BINARY="$SCRIPT_DIR/../target/debug/svault"
    BUILD_ARGS="-p svault-cli -q"
fi

echo "Checking svault binary ($([ "$RELEASE" = true ] && echo release || echo debug))..."
if [ ! -f "$BINARY" ]; then
    echo "Building svault..."
    cd "$SCRIPT_DIR/.."
    cargo build $BUILD_ARGS
fi

# Check for exiftool (used by some tests)
echo "Checking exiftool..."
if ! command -v exiftool &> /dev/null; then
    echo "Warning: exiftool is not installed. Some tests may fail."
    echo "Install it with: sudo apt install libimage-exiftool-perl  (Debian/Ubuntu)"
    echo "              or: brew install exiftool                    (macOS)"
fi

# Run tests (RAMDisk is managed by Python fixtures)
echo "Running tests..."
exec "$PYTEST" "${PYTEST_ARGS[@]}"
