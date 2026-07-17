#!/bin/bash
#
# run-tests.sh - Generate kv test results for community contribution
#
# =============================================================================
# BEFORE SUBMITTING - PLEASE READ!
# =============================================================================
#
# The goal of test results is to document how kv works on DIFFERENT systems.
# Before submitting, check the existing results in test_results/:
#
#   See: test_results/ directory or the project GitHub repository
#
# ONLY submit if your system is:
#   - A different architecture (ARM, RISC-V, x86, etc.)
#   - A unique SBC or embedded board not already covered
#   - An unusual configuration with interesting hardware
#   - A system where kv fails or shows unexpected behavior
#
# DON'T submit if:
#   - A very similar system is already in test_results/
#   - It's just another x86_64 laptop with nothing special
#   - Your output looks identical to an existing test
#
# =============================================================================
# PRIVACY NOTICE
# =============================================================================
#
# This script collects ONLY hardware information. We do NOT collect:
#   - Hostnames, usernames, or command history
#   - File contents or environment variables
#   - Any personally identifiable information (PII)
#
# MAC and IP addresses are automatically redacted from the output.
# Always run as a normal user (NOT root). Review output before submitting.
# Submitted results may be published publicly.
#
# =============================================================================
# USAGE
# =============================================================================
#
#   ./scripts/run-tests.sh <platform_id>                      Local machine
#   ./scripts/run-tests.sh remote <host> <arch> <platform_id> Remote via SSH
#   ./scripts/run-tests.sh --help                             Show this help
#
# Platform ID examples: MILKV_DUOS, RASPBERRY_PI_4B, THINKPAD_E14_GEN5
#
# =============================================================================

set -e

# Change to project root (parent of scripts/)
cd "$(dirname "$0")/.."

VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
DATE=$(date -u +"%Y-%m-%d %H:%M:%S UTC")
OUTPUT_DIR="test_results"

# Terminal colors (if interactive)
if [ -t 1 ]; then
    GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; RESET='\033[0m'
else
    GREEN=''; YELLOW=''; RED=''; RESET=''
fi

log_info()  { echo -e "${GREEN}[INFO]${RESET} $1"; }
log_warn()  { echo -e "${YELLOW}[WARN]${RESET} $1"; }
log_error() { echo -e "${RED}[ERROR]${RESET} $1"; }

# Sanitize output file - remove MAC and IP addresses for privacy
sanitize_output() {
    local file="$1"

    # MAC addresses in JSON (handles pretty-printed with spaces)
    sed -i 's/"mac_address": *"[^"]*"/"mac_address": "**:**:**:**:**:**"/g' "$file"

    # MAC addresses in text: MAC=xx:xx:xx:xx:xx:xx
    sed -i 's/MAC=[^ ]*/MAC=**:**:**:**:**:**/g' "$file"

    # IPv4 in JSON: "ip": "x.x.x.x"
    sed -i 's/"ip": *"[^"]*"/"ip": "*.*.*.*"/g' "$file"

    # IPv4 addresses as standalone quoted strings (in arrays)
    # Match "x.x.x.x" where x is 1-3 digits
    sed -i 's/"\([0-9]\{1,3\}\.\)\{3\}[0-9]\{1,3\}"/"*.*.*.*"/g' "$file"

    # IPv6 addresses as standalone quoted strings (in arrays)
    # Match quoted strings with colons that look like IPv6
    sed -i 's/"[0-9a-fA-F:]\{10,\}[^"]*"/"*:*:*:*"/g' "$file"

    # IPv4 in text: IP=x.x.x.x
    sed -i 's/IP=[0-9][0-9.]*[0-9]/IP=*.*.*.*/' "$file"
}

# -----------------------------------------------------------------------------
# System Info Header (plain text, no JSON)
# -----------------------------------------------------------------------------

gather_system_info() {
    echo "Kernel:        $(uname -r 2>/dev/null || echo 'unknown')"
    echo "Architecture:  $(uname -m 2>/dev/null || echo 'unknown')"

    # OS type
    if [ -f /etc/os-release ]; then
        echo "OS:            $(grep PRETTY_NAME /etc/os-release 2>/dev/null | cut -d'"' -f2)"
    fi

    # x86: DMI info (use || true to not fail with set -e)
    [ -f /sys/class/dmi/id/sys_vendor ] && \
        echo "Vendor:        $(cat /sys/class/dmi/id/sys_vendor 2>/dev/null)" || true
    [ -f /sys/class/dmi/id/product_name ] && \
        echo "Product:       $(cat /sys/class/dmi/id/product_name 2>/dev/null)" || true

    # ARM/RISC-V: Device tree
    [ -f /sys/firmware/devicetree/base/model ] && \
        echo "DT Model:      $(cat /sys/firmware/devicetree/base/model 2>/dev/null | tr -d '\0')" || true
}

# Same for remote (embedded in heredoc)
gather_system_info_remote() {
    ssh "$1" bash << 'EOF'
echo "Kernel:        $(uname -r 2>/dev/null || echo 'unknown')"
echo "Architecture:  $(uname -m 2>/dev/null || echo 'unknown')"
[ -f /etc/os-release ] && echo "OS:            $(grep PRETTY_NAME /etc/os-release 2>/dev/null | cut -d'"' -f2)" || true
[ -f /sys/class/dmi/id/sys_vendor ] && echo "Vendor:        $(cat /sys/class/dmi/id/sys_vendor 2>/dev/null)" || true
[ -f /sys/class/dmi/id/product_name ] && echo "Product:       $(cat /sys/class/dmi/id/product_name 2>/dev/null)" || true
[ -f /sys/firmware/devicetree/base/model ] && echo "DT Model:      $(cat /sys/firmware/devicetree/base/model 2>/dev/null | tr -d '\0')" || true
EOF
}

# -----------------------------------------------------------------------------
# Local Test
# -----------------------------------------------------------------------------

run_local() {
    local platform_id="$1"
    local output_file="${OUTPUT_DIR}/TEST_V${VERSION}_${platform_id}.txt"

    mkdir -p "$OUTPUT_DIR"
    log_info "Running tests for: $platform_id"
    log_info "Output: $output_file"

    # Build first
    cargo build --release 2>&1

    {
        echo "=============================================="
        echo "KV TEST RESULTS"
        echo "=============================================="
        echo "Version:       $VERSION"
        echo "Platform:      $platform_id"
        echo "Date:          $DATE"
        echo "Binary size:   $(ls -lh target/release/kv | awk '{print $5}')"
        echo ""
        gather_system_info
        echo "=============================================="
        echo ""

        echo "=============================================="
        echo "DEBUG OUTPUT (from kv snapshot -vp)"
        echo "=============================================="
        echo ""
        echo "Shows what files kv reads from /sys and /proc."
        echo "FAIL lines indicate missing/inaccessible files (often expected)."
        echo ""

        # Run snapshot with debug: stderr (debug) passes through, stdout (json) to temp file
        KV_DEBUG=1 ./target/release/kv snapshot -vp >/tmp/kv_snapshot_$$.json || true

        echo ""
        echo "=============================================="
        echo "SNAPSHOT OUTPUT (kv snapshot -vp)"
        echo "=============================================="
        echo ""
        cat /tmp/kv_snapshot_$$.json 2>/dev/null || echo "(snapshot failed)"
        rm -f /tmp/kv_snapshot_$$.json
        echo ""

        echo "=============================================="
        echo "END OF TEST RESULTS"
        echo "=============================================="

    } > "$output_file" 2>&1

    # Remove sensitive data (MAC/IP addresses)
    sanitize_output "$output_file"

    log_info "Results written to: $output_file"
    echo ""
    log_warn "Before submitting, please check existing test_results/ files."
    log_warn "Only submit if your system is unique or shows new behavior!"
}

# -----------------------------------------------------------------------------
# Remote Test
# -----------------------------------------------------------------------------

run_remote() {
    local remote_host="$1"
    local arch="$2"
    local platform_id="$3"
    local output_file="${OUTPUT_DIR}/TEST_V${VERSION}_${platform_id}.txt"

    mkdir -p "$OUTPUT_DIR"

    # Determine target triple
    local target=""
    local dt_feature=""
    case "$arch" in
        riscv64)      target="riscv64gc-unknown-linux-gnu"; dt_feature="--features dt" ;;
        arm64|aarch64) target="aarch64-unknown-linux-gnu"; dt_feature="--features dt" ;;
        arm)          target="arm-unknown-linux-gnueabihf"; dt_feature="--features dt" ;;
        ppc64|ppc64le) target="powerpc64le-unknown-linux-gnu"; dt_feature="--features dt" ;;
        mips|mipsel)  target="mipsel-unknown-linux-gnu"; dt_feature="--features dt" ;;
        x86_64)       target="x86_64-unknown-linux-gnu" ;;
        x86)          target="i686-unknown-linux-gnu" ;;
        *)
            log_error "Unknown arch: $arch (use: x86_64, x86, arm64, arm, riscv64, ppc64, mips)"
            exit 1
            ;;
    esac

    log_info "Building for $target..."
    cargo build --release -Zbuild-std=core -Zbuild-std-features=optimize_for_size \
        --target "$target" $dt_feature 2>&1

    local binary="target/$target/release/kv"
    [ ! -f "$binary" ] && { log_error "Binary not found: $binary"; exit 1; }

    log_info "Copying binary to $remote_host..."
    cat "$binary" | ssh "$remote_host" "cat > /tmp/kv && chmod +x /tmp/kv"

    log_info "Running tests on $remote_host..."

    {
        echo "=============================================="
        echo "KV TEST RESULTS (REMOTE)"
        echo "=============================================="
        echo "Version:       $VERSION"
        echo "Platform:      $platform_id"
        echo "Date:          $DATE"
        echo "Target:        $target"
        echo "Binary size:   $(ls -lh "$binary" | awk '{print $5}')"
        echo ""
        gather_system_info_remote "$remote_host"
        echo "=============================================="
        echo ""

        echo "=============================================="
        echo "DEBUG OUTPUT (from kv snapshot -vp)"
        echo "=============================================="
        echo ""
        echo "Shows what files kv reads from /sys and /proc."
        echo "FAIL lines indicate missing/inaccessible files (often expected)."
        echo ""

        # Run snapshot with debug on remote: stderr (debug) passes through, stdout (json) to temp file
        ssh "$remote_host" "KV_DEBUG=1 /tmp/kv snapshot -vp >/tmp/kv_snapshot.json" || true

        echo ""
        echo "=============================================="
        echo "SNAPSHOT OUTPUT (kv snapshot -vp)"
        echo "=============================================="
        echo ""
        ssh "$remote_host" "cat /tmp/kv_snapshot.json 2>/dev/null; rm -f /tmp/kv_snapshot.json" || echo "(snapshot failed)"
        echo ""

        echo "=============================================="
        echo "END OF TEST RESULTS"
        echo "=============================================="

    } > "$output_file" 2>&1

    # Cleanup
    ssh "$remote_host" "rm -f /tmp/kv" 2>/dev/null || true

    # Remove sensitive data (MAC/IP addresses)
    sanitize_output "$output_file"

    log_info "Results written to: $output_file"
    echo ""
    log_warn "Before submitting, please check existing test_results/ files."
    log_warn "Only submit if your system is unique or shows new behavior!"
}

# -----------------------------------------------------------------------------
# Help
# -----------------------------------------------------------------------------

print_help() {
    cat << 'EOF'
run-tests.sh - Generate kv test results

BEFORE SUBMITTING:
    Check existing results at test_results/ or on GitHub.
    Only submit if your system is unique or shows different behavior!

USAGE:
    ./scripts/run-tests.sh <platform_id>                      Local
    ./scripts/run-tests.sh remote <host> <arch> <platform_id> Remote via SSH
    ./scripts/run-tests.sh --help                             This help

PLATFORM_ID EXAMPLES:
    MILKV_DUOS, RASPBERRY_PI_4B, JETSON_AGX_ORIN, THINKPAD_E14_GEN5

REMOTE ARCHITECTURES:
    riscv64, arm64, arm, x86_64

EXAMPLES:
    ./scripts/run-tests.sh THINKPAD_E14_GEN5
    ./scripts/run-tests.sh remote pi@192.168.1.100 arm64 RASPBERRY_PI_4B
    ./scripts/run-tests.sh remote me@192.168.42.1 riscv64 MILKV_DUOS

OUTPUT:
    test_results/TEST_V<version>_<platform_id>.txt

    The output contains:
    1. System info header (kernel, arch, model)
    2. Debug output showing what files kv reads
    3. Pretty-printed JSON snapshot of all hardware

PRIVACY:
    MAC and IP addresses are automatically redacted from output.

EOF
}

# -----------------------------------------------------------------------------
# Main
# -----------------------------------------------------------------------------

case "${1:-}" in
    -h|--help|help|"")
        print_help
        [ -z "${1:-}" ] && log_error "Platform ID required"
        exit 0
        ;;
    remote)
        [ -z "${4:-}" ] && { log_error "Usage: $0 remote <host> <arch> <platform_id>"; exit 1; }
        run_remote "$2" "$3" "$4"
        ;;
    *)
        echo "$1" | grep -qE '^[A-Za-z0-9_]+$' || {
            log_error "Invalid platform_id: use only letters, numbers, underscores"
            exit 1
        }
        run_local "$1"
        ;;
esac
