# Contributing to kv

Thanks for your interest in kv! Whether you're new to open source or a
seasoned contributor, we're happy to have you.

**Ways to contribute:**

- **Test results** - Run kv on your hardware and share the output
- **Bug reports** - Something not working? Let us know
- **Feature requests** - Ideas for improvements (use [Discussions](../../discussions))
- **Code** - Fix bugs, add features, improve documentation

New to GitHub? Don't worry - we'll walk you through it below.

## Submitting Test Results

We want kv to work on as many systems as possible. Test results help us find
parsing issues, missing features, and edge cases.

### What We're Looking For

Before submitting, check `test_results/` to see if similar hardware is covered.
We especially want results from:

- **Unusual embedded boards** - Custom ARM/RISC-V SoCs, industrial computers
- **Exotic hardware** - FPGA soft-cores, vintage architectures
- **Edge cases** - Systems with hundreds of devices, unusual kernel configs
- **Non-standard setups** - Buildroot, Yocto, minimal busybox systems
- **Failures** - Systems where kv doesn't work correctly

Don't submit if your system is nearly identical to one we already have.

### Privacy Notice

The test script collects **only hardware information**. It does NOT collect:
- Hostnames, usernames, or command history
- File contents or environment variables
- Any personally identifiable information

**MAC and IP addresses are automatically redacted** from the output (replaced with `*`).

Always run as a normal user (not root) and review the output before submitting.

### Running the Test Script

```bash
# Local machine - describe your hardware in the platform ID
./scripts/run-tests.sh RASPBERRY_PI_4B
./scripts/run-tests.sh JETSON_AGX_ORIN
./scripts/run-tests.sh THINKPAD_E14_GEN5

# Remote embedded system via SSH
./scripts/run-tests.sh remote pi@192.168.1.100 arm64 RASPBERRY_PI_4B
./scripts/run-tests.sh remote root@192.168.42.1 riscv64 MILKV_DUOS
```

Output: `test_results/TEST_V<version>_<platform_id>.txt`

### How to Submit (Step by Step)

**Option 1: Open a GitHub Issue (easiest - no git knowledge needed)**

1. Run the test script on your hardware
2. Open the output file and copy its contents (or keep the file ready to attach)
3. Go to the [Issues page](../../issues)
4. Click the green **"New issue"** button
5. Give it a title like: `Test results: Raspberry Pi 4B` or `Test results: BeagleBone Black`
6. In the description box:
   - Briefly describe your hardware (board name, CPU, any special setup)
   - Paste the test output, or drag-and-drop the `.txt` file to attach it
7. Click **"Submit new issue"**

That's it! We'll take it from there.

**Option 2: Submit a Pull Request (if you know git)**

1. Fork this repository (click "Fork" button at top right)
2. Clone your fork: `git clone https://github.com/walruscraft/kv.git`
3. Add your test file to `test_results/`
4. Commit and push: `git add . && git commit -m "Add test results for MY_BOARD" && git push`
5. Go to your fork on GitHub and click **"Contribute" -> "Open pull request"**

**Option 3: Start a Discussion (for questions or ideas)**

Have a question? Want to suggest a feature? Not sure if something is a bug?

1. Go to the [Discussions page](../../discussions)
2. Click **"New discussion"**
3. Pick a category (Q&A, Ideas, etc.)
4. Write your question or idea and submit

Discussions are great for back-and-forth conversation before opening a formal issue.

### Debug Mode

If kv has issues on your hardware, run with debug mode to see what's happening:

```bash
KV_DEBUG=1 kv pci      # Shows what files are being read
kv pci -D              # Same thing, via flag
```

## Contributing Code

### Development Setup

```bash
git clone https://github.com/walruscraft/kv.git
cd kv
cargo build
./build.sh && ./scripts/test.sh   # Build and run integration tests
```

### Code Style

- **Minimal crates** - only origin (startup), rustix (syscalls), itoa (number formatting)
- **Human-readable comments** - Not cold/dry generated text
- **Small focused functions** - Each does one thing well
- **Explicit types** - For public structs and complex signatures
- **JSON fields** - Use `lowercase_with_underscores`
- **No unsafe** - Unless absolutely necessary (document if used)

### Testing

Two layers:

```bash
./scripts/unit-test.sh  # Unit tests (parsers, formatters) on the host
./build.sh              # Build release binary (static non-PIE, smallest)
./build.sh --pie        # Static-PIE variant (ASLR, not on x86/mipsel yet)
./scripts/test.sh       # Integration tests against the built binary
./scripts/test-cross.sh # Build all targets, QEMU smoke tests
```

Unit tests live in the library modules (`cargo test` via the wrapper - the
wrapper clears RUSTFLAGS because the release link flags would break the std
test harness). Integration tests verify all subcommands, JSON output,
filters, verbose mode, and human-readable sizes against real /sys and /proc.

> [!TIP]
> Chuck Norris tests in production.

### CI/CD

**Continuous Integration** (`.github/workflows/ci.yml`)

Runs on every push to `main` and on pull requests:

1. **Build job** - Builds release binaries for all 7 architectures:
   - Native: x86_64, i686
   - Cross-compiled: aarch64, arm, riscv64
   - Runs integration tests via QEMU for cross-compiled targets
   - Uploads binaries as artifacts (available for 90 days)

**Release** (`.github/workflows/release.yml`)

Runs when a version tag is pushed (e.g., `git tag v0.5.0 && git push --tags`):

1. Builds all 7 architecture binaries
2. Runs smoke tests (native and via QEMU)
3. Creates a GitHub Release with:
   - All binaries attached (`kv-x86_64`, `kv-arm64`, etc.)
   - Auto-generated release notes from commits

**Manual triggers**: Both workflows can be run manually from the GitHub Actions tab.

### Cross-Compilation

Cargo aliases for common targets (all use gnu targets with build-std):

```bash
cargo x86_64   # x86_64-unknown-linux-gnu
cargo x86      # i686-unknown-linux-gnu
cargo arm64    # aarch64-unknown-linux-gnu (includes dt)
cargo aarch64  # same as arm64
cargo arm      # arm-unknown-linux-gnueabihf (includes dt)
cargo riscv64  # riscv64gc-unknown-linux-gnu (includes dt)
cargo ppc64    # powerpc64le-unknown-linux-gnu (includes dt)
cargo mips     # mipsel-unknown-linux-gnu (includes dt)
```

Prerequisites (Debian/Ubuntu):

```bash
# Nightly toolchain required for build-std
rustup default nightly

# Cross-linkers (from an x86_64 host)
sudo apt install gcc-aarch64-linux-gnu gcc-arm-linux-gnueabihf gcc-riscv64-linux-gnu gcc-powerpc64le-linux-gnu gcc-mipsel-linux-gnu

# QEMU for testing (optional)
sudo apt install qemu-user-static
```

**Building from a non-x86 host (e.g., aarch64):**

The x86_64 and i686 targets don't have a `linker` set in `.cargo/config.toml`
because CI runs on x86_64 where `cc` works natively. On other hosts, you need
cross-linkers for those two as well:

```bash
# Install x86 cross-linkers
sudo apt install gcc-x86-64-linux-gnu gcc-i686-linux-gnu

# Then either set env vars when building:
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc cargo x86_64
CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_LINKER=i686-linux-gnu-gcc cargo x86

# Or export them for the session:
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc
export CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_LINKER=i686-linux-gnu-gcc
cargo x86_64
cargo x86
```

Alternatively, you can add the linkers to `.cargo/config.toml` directly:

```toml
[target.x86_64-unknown-linux-gnu]
linker = "x86_64-linux-gnu-gcc"

[target.i686-unknown-linux-gnu]
linker = "i686-linux-gnu-gcc"
```

This works everywhere but requires the cross-compiler packages even on x86 hosts.

### Security Guidelines

kv runs on untrusted systems where inputs cannot be trusted. All code must:

1. **Never trust input** - Command-line args, file contents, symlinks - assume hostile
2. **Bound all inputs** - String lengths, recursion depth, file counts need limits
3. **Fail safely** - Return `Option`/`Result`, don't panic. Missing data is normal.
4. **Escape outputs** - All strings in JSON go through proper escaping
5. **Validate paths** - Sanitize user-provided paths before joining with base paths
6. **Test edge cases** - Empty inputs, very long inputs, malformed data

See the "Security" section in README.md for what's already implemented.

### Commit Guidelines

- Use conventional commit style
- Commit when feature is complete, even if not fully testable
- Include comments about test limitations where relevant

### Pull Request Process

1. **Fork** the repository (click "Fork" at top right of the GitHub page)
2. **Clone** your fork: `git clone https://github.com/walruscraft/kv.git`
3. **Create a branch**: `git checkout -b my-feature`
4. **Make your changes** and test them: `./scripts/unit-test.sh && ./build.sh && ./scripts/test.sh`
5. **Commit**: `git add . && git commit -m "Description of changes"`
6. **Push**: `git push origin my-feature`
7. **Open a PR**: Go to your fork on GitHub, click "Contribute" -> "Open pull request"

Not sure about something? Open a [Discussion](../../discussions) first - we're happy to help!

## Need Help?

- **Questions about kv?** - [Start a Discussion](../../discussions)
- **Found a bug?** - [Open an Issue](../../issues/new)
- **Want to contribute but not sure where to start?** - Look for issues labeled `good first issue`
- **Something else?** - Email the author at leonvak@gmail.com

Don't be shy - there are no stupid questions!

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
