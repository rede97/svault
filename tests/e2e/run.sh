#!/bin/bash
# Run E2E tests with proper environment isolation
#
# Usage:
#   ./run.sh                           # Run all tests (debug build), exclude FUSE
#   ./run.sh --fuse                    # Include FUSE tests
#   ./run.sh --release                 # Run with release build
#   ./run.sh --ramdisk-size 512m       # Use 512MB RAMDisk
#   ./run.sh --ramdisk-size 1g --cleanup  # Use 1GB RAMDisk and cleanup after
#   ./run.sh -v -k test_import         # Verbose, only matching tests

set -e

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$SCRIPT_DIR"

# Clean environment to avoid ROS conflicts
unset PYTHONPATH
unset PYTEST_PLUGINS

# Use venv python directly
if command -v uv &> /dev/null; then
    PYTHON="uv run python"
    PYTEST="uv run python -m pytest"
elif [ -f "$SCRIPT_DIR/.venv/bin/python" ]; then
    PYTHON="$SCRIPT_DIR/.venv/bin/python"
    PYTEST="$SCRIPT_DIR/.venv/bin/python -m pytest"
else
    echo "Error: No Python environment found. Run 'uv sync' first."
    exit 1
fi

# Parse options
PYTEST_ARGS=()
RELEASE=false
RUN_FUSE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --fuse)
            RUN_FUSE=true
            shift
            ;;
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
        -h|--help)
            echo "Usage: ./run.sh [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --fuse               Include FUSE tests (default: excluded)"
            echo "  --release            Use release build of svault"
            echo "  --ramdisk-size SIZE  Set RAMDisk size (e.g., 512m, 1g)"
            echo "  --ramdisk-path PATH  Set RAMDisk mount path"
            echo "  --cleanup            Cleanup RAMDisk after tests"
            echo "  -v                   Verbose output"
            echo "  -k EXPRESSION        Only run tests matching expression"
            echo "  -h, --help           Show this help"
            echo ""
            echo "Examples:"
            echo "  ./run.sh                           # Run all tests (excluding FUSE)"
            echo "  ./run.sh --fuse                    # Run all tests including FUSE"
            echo "  ./run.sh -v -k test_import         # Verbose, only import tests"
            echo "  ./run.sh --release --fuse          # Release build with FUSE tests"
            exit 0
            ;;
        *)
            PYTEST_ARGS+=("$1")
            shift
            ;;
    esac
done

# Ensure binary is built
if [ "$RELEASE" = true ]; then
    BINARY="$PROJECT_ROOT/target/release/svault"
    BUILD_ARGS="--release -p svault -q"
else
    BINARY="$PROJECT_ROOT/target/debug/svault"
    BUILD_ARGS="-p svault -q"
fi

echo "Checking svault binary ($([ "$RELEASE" = true ] && echo release || echo debug))..."
if [ ! -f "$BINARY" ]; then
    echo "Building svault..."
    cd "$PROJECT_ROOT"
    cargo build $BUILD_ARGS
fi

# Check for exiftool (used by some tests)
echo "Checking exiftool..."
if ! command -v exiftool &> /dev/null; then
    echo "Warning: exiftool is not installed. Some tests may fail."
    echo "Install it with: sudo apt install libimage-exiftool-perl  (Debian/Ubuntu)"
    echo "              or: brew install exiftool                    (macOS)"
fi

# Default: exclude FUSE tests unless --fuse specified
if [ "$RUN_FUSE" = false ]; then
    PYTEST_ARGS+=("--ignore=fuse_tests/")
    echo "Running tests (excluding FUSE tests, use --fuse to include)..."
else
    echo "Running tests (including FUSE tests)..."
fi

# Run tests
if command -v uv &> /dev/null; then
    uv run python -m pytest "${PYTEST_ARGS[@]}"
else
    "$PYTHON" -m pytest "${PYTEST_ARGS[@]}"
fi
