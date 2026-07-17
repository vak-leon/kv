#!/bin/sh
# Run kv's unit tests on the host.
#
# RUSTFLAGS is cleared on purpose: .cargo/config.toml sets link flags like
# -nostartfiles for the no_std release binary, and those would break the
# std test harness. An empty RUSTFLAGS overrides them for this run.
#
# --lib only: the kv binary itself is no_std and can't be built as a test
# harness. Integration testing of the binary lives in scripts/test.sh.
set -e

# --features dt: on devicetree architectures this compiles the real dt
# module (and its tests). Elsewhere the stub compiles. Both must build.
cd "$(dirname "$0")/.."
RUSTFLAGS= exec cargo test --lib --features dt "$@"
