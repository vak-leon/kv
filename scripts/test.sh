#!/bin/sh
# Integration tests for kv
# Run after building: ./build.sh && ./scripts/test.sh

set -e

# Default to the host-target release binary (what ./build.sh produces);
# override with KV=path/to/kv for cross binaries.
KV="${KV:-./target/$(rustc -vV | sed -n 's/^host: //p')/release/kv}"

# Colors for output (if terminal supports it)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    NC=''
fi

PASSED=0
FAILED=0

pass() {
    PASSED=$((PASSED + 1))
    printf "${GREEN}PASS${NC}: %s\n" "$1"
}

fail() {
    FAILED=$((FAILED + 1))
    printf "${RED}FAIL${NC}: %s\n" "$1"
}

# Check binary exists
if [ ! -x "$KV" ]; then
    echo "Error: $KV not found or not executable"
    echo "Run ./build.sh first"
    exit 1
fi

echo "Testing: $KV"
echo "---"

# Test: version flag
if $KV --version | grep -q "kv [0-9]"; then
    pass "--version shows version"
else
    fail "--version shows version"
fi

# Test: help flag
if $KV --help | grep -q "USAGE"; then
    pass "--help shows usage"
else
    fail "--help shows usage"
fi

# Test: no args shows error
if $KV 2>&1 | grep -q "no subcommand"; then
    pass "no args shows error"
else
    fail "no args shows error"
fi

# Test: unknown subcommand
if $KV badcmd 2>&1 | grep -q "unknown subcommand"; then
    pass "unknown subcommand shows error"
else
    fail "unknown subcommand shows error"
fi

# Test: cpu subcommand runs
if $KV cpu | grep -q "LOGICAL_CPUS"; then
    pass "cpu runs and shows output"
else
    fail "cpu runs and shows output"
fi

# Test: cpu json output
if $KV cpu -j | grep -q '"logical_cpus"'; then
    pass "cpu -j produces JSON"
else
    fail "cpu -j produces JSON"
fi

# Test: mem subcommand runs
if $KV mem | grep -q "MEM_TOTAL"; then
    pass "mem runs and shows output"
else
    fail "mem runs and shows output"
fi

# Test: mem json output
if $KV mem -j | grep -q '"mem_total'; then
    pass "mem -j produces JSON"
else
    fail "mem -j produces JSON"
fi

# Test: mem human readable
if $KV mem -h | grep -qE "[0-9]+(\.[0-9]+)?[KMGT]"; then
    pass "mem -h shows human readable sizes"
else
    fail "mem -h shows human readable sizes"
fi

# Test: pci subcommand runs (may have no devices in some environments)
if $KV pci 2>&1 | grep -qE "(BDF=|no PCI)"; then
    pass "pci runs"
else
    fail "pci runs"
fi

# Test: block subcommand runs
if $KV block 2>&1 | grep -qE "(NAME=|no block)"; then
    pass "block runs"
else
    fail "block runs"
fi

# Test: net subcommand runs
if $KV net 2>&1 | grep -qE "(NAME=|no network)"; then
    pass "net runs"
else
    fail "net runs"
fi

# Test: mounts subcommand runs
if $KV mounts | grep -q "TARGET="; then
    pass "mounts runs and shows output"
else
    fail "mounts runs and shows output"
fi

# Test: thermal subcommand runs
if $KV thermal 2>&1 | grep -qE "(SENSOR=|TEMP=|no.*(thermal|temperature|sensor))"; then
    pass "thermal runs"
else
    fail "thermal runs"
fi

# Test: power subcommand runs
if $KV power 2>&1 | grep -qE "(NAME=|no power)"; then
    pass "power runs"
else
    fail "power runs"
fi

# Test: usb subcommand runs
if $KV usb 2>&1 | grep -qE "(NAME=|no USB)"; then
    pass "usb runs"
else
    fail "usb runs"
fi

# Test: snapshot produces JSON
if $KV snapshot | grep -q '"kv_version"'; then
    pass "snapshot produces JSON with version"
else
    fail "snapshot produces JSON with version"
fi

# Test: combined flags -jv
if $KV cpu -jv | grep -q '"architecture"'; then
    pass "combined flags -jv work"
else
    fail "combined flags -jv work"
fi

# Test: filter flag
if $KV mounts -f tmpfs | grep -q "tmpfs"; then
    pass "filter -f works"
else
    # tmpfs might not exist, check if filter runs at all
    if $KV mounts -f nonexistent 2>&1 | grep -qE "(no matching|TARGET=)"; then
        pass "filter -f works (no tmpfs found)"
    else
        fail "filter -f works"
    fi
fi

# Test: pretty JSON
if $KV mem -jp | grep -q '  "'; then
    pass "pretty JSON -jp has indentation"
else
    fail "pretty JSON -jp has indentation"
fi

# Test: verbose mode adds fields
CPU_NORMAL=$($KV cpu | wc -w)
CPU_VERBOSE=$($KV cpu -v | wc -w)
if [ "$CPU_VERBOSE" -gt "$CPU_NORMAL" ]; then
    pass "verbose -v adds more fields"
else
    fail "verbose -v adds more fields"
fi

# Test: cpu count matches the system (guards against truncated /proc/cpuinfo)
NPROC=$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN)
if $KV cpu | grep -q "LOGICAL_CPUS=$NPROC "; then
    pass "cpu LOGICAL_CPUS matches nproc ($NPROC)"
else
    fail "cpu LOGICAL_CPUS matches nproc ($NPROC, got: $($KV cpu | grep -o 'LOGICAL_CPUS=[0-9]*'))"
fi

# Test: mounts shows every mount (guards against truncated /proc/self/mounts)
MOUNT_LINES=$(wc -l < /proc/self/mounts)
KV_MOUNTS=$($KV mounts | grep -c "TARGET=" || true)
if [ "$KV_MOUNTS" -eq "$MOUNT_LINES" ]; then
    pass "mounts shows all $MOUNT_LINES mounts"
else
    fail "mounts shows all mounts (kernel has $MOUNT_LINES, kv shows $KV_MOUNTS)"
fi

# Test: pci -v resolves driver symlinks (only if the system has any)
if ls /sys/bus/pci/devices/*/driver >/dev/null 2>&1; then
    if $KV pci -v | grep -q "DRIVER="; then
        pass "pci -v shows DRIVER for bound devices"
    else
        fail "pci -v shows DRIVER for bound devices"
    fi
else
    pass "pci -v shows DRIVER (skipped: no bound PCI drivers)"
fi

# Test: JSON never renders numa_node -1 as a huge unsigned value
if $KV pci -jv | grep -q "18446744073709551615"; then
    fail "pci -jv renders numa_node -1 correctly"
else
    pass "pci -jv renders numa_node -1 correctly"
fi

# Test: -D enables debug output on stderr
if $KV mem -D 2>&1 >/dev/null | grep -q "\[DEBUG\]"; then
    pass "-D emits debug output on stderr"
else
    fail "-D emits debug output on stderr"
fi

# Test: KV_DEBUG=1 works like -D
if KV_DEBUG=1 $KV mem 2>&1 >/dev/null | grep -q "\[DEBUG\]"; then
    pass "KV_DEBUG=1 emits debug output"
else
    fail "KV_DEBUG=1 emits debug output"
fi

# Test: debug output does not leak into stdout (would corrupt JSON consumers)
if $KV mem -Dj 2>/dev/null | grep -q "\[DEBUG\]"; then
    fail "debug output stays out of stdout"
else
    pass "debug output stays out of stdout"
fi

echo "---"
echo "Results: $PASSED passed, $FAILED failed"

if [ "$FAILED" -gt 0 ]; then
    exit 1
fi
