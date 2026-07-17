#!/bin/bash
# Simple cross-compilation smoke test
# Builds available targets and runs them via QEMU

set -e

# Change to project root (parent of scripts/)
cd "$(dirname "$0")/.."

# Colors (if terminal supports them)
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

pass() { echo -e "${GREEN}PASS${NC}: $1"; }
fail() { echo -e "${RED}FAIL${NC}: $1"; }
skip() { echo -e "${YELLOW}SKIP${NC}: $1"; }

# Test a single target
# Args: alias target qemu [dynamic]
# If 4th arg is "dynamic", don't fail on dynamically-linked binary
test_target() {
    local alias=$1
    local target=$2
    local qemu=$3
    local allow_dynamic=$4
    local binary="target/${target}/release/kv"

    echo "=== $alias ($target) ==="

    # Build
    if ! cargo "$alias" 2>/dev/null; then
        skip "$alias - build failed (missing cross-linker?)"
        return
    fi

    if [ ! -f "$binary" ]; then
        fail "$alias - binary not found"
        return
    fi

    # Check it's static (or allow dynamic for targets without musl)
    if ldd "$binary" 2>&1 | grep -qE "(not a dynamic|statically linked)"; then
        pass "$alias - static binary"
    elif [ "$allow_dynamic" = "dynamic" ]; then
        pass "$alias - dynamic binary (musl unavailable)"
    else
        fail "$alias - not static"
    fi

    # Run via QEMU (or native for x86_64)
    if [ -z "$qemu" ]; then
        # Native execution
        if "$binary" --version >/dev/null 2>&1; then
            pass "$alias - runs natively"
        else
            fail "$alias - native execution failed"
        fi
    elif [ "$qemu" = "skip" ]; then
        skip "$alias - QEMU run skipped (dynamic binary needs libs)"
    elif command -v "$qemu" >/dev/null 2>&1; then
        if "$qemu" "$binary" --version >/dev/null 2>&1; then
            pass "$alias - runs via $qemu"
        else
            fail "$alias - QEMU execution failed"
        fi
    else
        skip "$alias - $qemu not installed"
    fi

    # Show size
    local size=$(stat -c%s "$binary" 2>/dev/null || stat -f%z "$binary" 2>/dev/null)
    echo "     Size: $((size / 1024)) KB"
    echo
}

echo "Cross-compilation smoke test"
echo "============================"
echo

test_target "x86_64"  "x86_64-unknown-linux-gnu"      ""
test_target "x86"     "i686-unknown-linux-gnu"        "qemu-i386-static"
test_target "arm64"   "aarch64-unknown-linux-gnu"     "qemu-aarch64-static"
test_target "arm"     "arm-unknown-linux-gnueabihf"   "qemu-arm-static"
test_target "riscv64" "riscv64gc-unknown-linux-gnu"   "qemu-riscv64-static"
test_target "ppc64"   "powerpc64le-unknown-linux-gnu" "qemu-ppc64le-static"
test_target "mips"    "mipsel-unknown-linux-gnu"      "qemu-mipsel-static"

echo "Done."
