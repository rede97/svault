#!/bin/bash
# Build svault binary with maximum compatibility for older Linux distributions
# Uses cargo-zigbuild to target glibc 2.17 (CentOS 7 compatible)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

echo "Building svault for x86_64-unknown-linux-gnu.2.17 (CentOS 7 compatible)..."
echo "Output directory: target/centos"

cd "$PROJECT_ROOT"

# Check if cargo-zigbuild is installed
if ! command -v cargo-zigbuild &> /dev/null; then
    echo "Error: cargo-zigbuild is not installed."
    echo "Install it with: cargo install cargo-zigbuild"
    echo "And ensure zig is installed: https://ziglang.org/download/"
    exit 1
fi

# Build with zigbuild for maximum compatibility
cargo zigbuild --release \
    --target x86_64-unknown-linux-gnu.2.17 \
    --target-dir=target/centos

echo ""
echo "Build complete!"
echo "Binary location: target/centos/x86_64-unknown-linux-gnu/release/svault"
echo ""
echo "Checking binary compatibility:"
file "target/centos/x86_64-unknown-linux-gnu/release/svault"
ldd "target/centos/x86_64-unknown-linux-gnu/release/svault" 2>/dev/null | head -5 || echo "(ldd check skipped)"
