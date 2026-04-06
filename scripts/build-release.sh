#!/bin/bash
# Build svault release binaries
# Supports standard build and maximum compatibility build for older Linux distros

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --native       Build for current platform (default)"
    echo "  --centos       Build for CentOS 7 / old glibc (2.17+) compatibility"
    echo "  --all          Build all variants"
    echo "  --help         Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0                    # Native build"
    echo "  $0 --native           # Native build"
    echo "  $0 --centos           # CentOS compatible build using zigbuild"
    echo "  $0 --all              # Build all variants"
}

build_native() {
    echo "=== Building native release binary ==="
    cargo build --release
    echo "Binary: target/release/svault"
    file target/release/svault
}

build_centos() {
    echo "=== Building CentOS 7 compatible binary (glibc 2.17+) ==="
    
    if ! command -v cargo-zigbuild &> /dev/null; then
        echo "Error: cargo-zigbuild is not installed."
        echo "Install it with: cargo install cargo-zigbuild"
        echo "And ensure zig is installed: https://ziglang.org/download/"
        exit 1
    fi
    
    cargo zigbuild --release \
        --target x86_64-unknown-linux-gnu.2.17 \
        --target-dir=target/centos
    
    echo "Binary: target/centos/x86_64-unknown-linux-gnu/release/svault"
    file target/centos/x86_64-unknown-linux-gnu/release/svault
}

# Parse arguments
case "${1:-}" in
    --centos)
        build_centos
        ;;
    --native|"")
        build_native
        ;;
    --all)
        build_native
        echo ""
        build_centos
        ;;
    --help|-h)
        usage
        exit 0
        ;;
    *)
        echo "Unknown option: $1"
        usage
        exit 1
        ;;
esac

echo ""
echo "Build complete!"
