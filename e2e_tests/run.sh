#!/bin/bash
# Run E2E tests with proper environment isolation
#
# Usage:
#   ./run.sh                           # Run all tests
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

while [[ $# -gt 0 ]]; do
    case $1 in
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
echo "Checking svault binary..."
if [ ! -f "$SCRIPT_DIR/../target/release/svault" ]; then
    echo "Building svault..."
    cd "$SCRIPT_DIR/.."
    cargo build --release -p svault-cli -q
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
