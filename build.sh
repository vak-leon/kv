#!/bin/sh
# Build kv with minimal binary size.
#
# Usage:
#   ./build.sh           # static non-PIE build (default, smallest)
#   ./build.sh --pie     # static-PIE build (ASLR-capable, slightly larger)
#
# Defaults to the host architecture; set TARGET to cross-build, e.g.:
#   TARGET=aarch64-unknown-linux-gnu ./build.sh
set -e

TARGET="${TARGET:-$(rustc -vV | sed -n 's/^host: //p')}"

FEATURES=""
case "$TARGET" in
    aarch64-*|arm-*|riscv64*|powerpc64le-*|mipsel-*) FEATURES="dt" ;;
esac

PIE=0
for arg in "$@"; do
    case "$arg" in
        --pie) PIE=1 ;;
        *) echo "Unknown option: $arg (supported: --pie)"; exit 1 ;;
    esac
done

if [ "$PIE" = "1" ]; then
    # Static-PIE: PIC code, -static-pie link, and origin's
    # experimental-relocate (the `pie` feature) to apply relocations at
    # startup. RUSTFLAGS overrides the non-PIE defaults in
    # .cargo/config.toml. The flag list below must mirror it minus the
    # static/no-pie/norelro parts.
    #
    # Not every arch can do this yet: origin has no PIC-mode _start for
    # MIPS, and its i686 asm trips .cfi assembler errors under PIC.
    case "$TARGET" in
        i686-*|mipsel-*)
            echo "Error: --pie is not supported on $TARGET (origin limitation)"
            exit 1
            ;;
    esac
    RUSTFLAGS="-C link-arg=-nostartfiles -C link-arg=-static-pie \
        -C link-arg=-Wl,--gc-sections -C link-arg=-Wl,--build-id=none \
        -C force-unwind-tables=no -Z location-detail=none -Z fmt-debug=none"
    case "$TARGET" in
        powerpc64le-*|mipsel-*) RUSTFLAGS="$RUSTFLAGS --cfg=rustix_use_experimental_asm" ;;
    esac
    export RUSTFLAGS
    FEATURES="${FEATURES:+$FEATURES,}pie"
fi

cargo build --release \
    -Zbuild-std=core -Zbuild-std-features=optimize_for_size \
    --target "$TARGET" ${FEATURES:+--features "$FEATURES"}

# Strip the one leftover the compile flags don't cover (.comment metadata).
# Prefer the target-prefixed objcopy (installed with the cross gcc) since
# a plain non-multiarch objcopy can't read foreign-arch ELFs. Best-effort
# either way - it only saves a few hundred bytes.
BINARY="target/$TARGET/release/kv"
GNU_TRIPLE=$(echo "$TARGET" | sed 's/-unknown//; s/^riscv64gc/riscv64/')
if command -v "${GNU_TRIPLE}-objcopy" >/dev/null 2>&1; then
    "${GNU_TRIPLE}-objcopy" -R .comment "$BINARY"
elif command -v objcopy >/dev/null 2>&1; then
    objcopy -R .comment "$BINARY" 2>/dev/null \
        || echo "note: objcopy can't handle $TARGET binaries on this host, skipping .comment strip"
fi

ls -la "$BINARY"
