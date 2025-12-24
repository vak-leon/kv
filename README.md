# kv - Kernel View

[![Rust 2024](https://img.shields.io/badge/Rust_2024-≥1.85-black?style=for-the-badge&logo=rust&logoColor=white)](https://doc.rust-lang.org/edition-guide/rust-2024/)

A tiny, dependency-free system inspector for Linux.

Born from the frustration of SSHing into an embedded board and realizing
that lspci, lsusb, and all the rest of most essential info-oriented tools
are nowhere to be found. kv is a single static binary that tells you what
hardware you've got, reading directly from /sys and /proc like nature
intended.

## Installation

Pre-built static binaries are available from [GitHub Releases](https://github.com/vak-leon/kv/releases/latest).

### Download, Verify, Install

Each release includes SHA256 checksums for verification.

**x86_64** (most Linux servers and desktops):

```bash
curl -LO https://github.com/vak-leon/kv/releases/latest/download/kv-x64
curl -LO https://github.com/vak-leon/kv/releases/latest/download/kv-x64.sha256
sha256sum -c kv-x64.sha256
mv kv-x64 kv && chmod +x kv
./kv --version
```

**ARM64** (Raspberry Pi 4/5, Jetson, Apple Silicon VMs):

```bash
curl -LO https://github.com/vak-leon/kv/releases/latest/download/kv-arm64
curl -LO https://github.com/vak-leon/kv/releases/latest/download/kv-arm64.sha256
sha256sum -c kv-arm64.sha256
mv kv-arm64 kv && chmod +x kv
./kv --version
```

**Other architectures:**
Use `kv-x86` (32-bit PC), `kv-arm` (32-bit ARM), `kv-riscv64` (64-bit RISC-V), or `kv-ppc` (PowerPC).
Each has corresponding `.sha256`, `.sig`, and `.crt` files.

### Verify Signatures (optional)

Releases are signed with [cosign](https://github.com/sigstore/cosign) keyless signing via GitHub Actions OIDC.
This proves the binary was built by the official release workflow, not a compromised maintainer machine.

```bash
# Install cosign (https://docs.sigstore.dev/cosign/system_config/installation/)
# Then verify:
cosign verify-blob kv-x64 \
  --signature kv-x64.sig \
  --certificate kv-x64.crt \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp 'https://github.com/vak-leon/kv/\.github/workflows/release\.yml@refs/tags/v.*'
```

**Quick install (skip verification):**

```bash
curl -Lo kv https://github.com/vak-leon/kv/releases/latest/download/kv-x64 && chmod +x kv
```

**Copy to an embedded board:**

```bash
scp kv-arm64 user@board:/tmp/kv
ssh user@board chmod +x /tmp/kv
ssh user@board /tmp/kv snapshot -jp
```

> [!NOTE]
> If using a Windows machine to copy the file to the target, `chmod` it on the target to make it runnable.

## Usage

```bash
kv pci          # PCI devices
kv usb          # USB devices
kv block        # Disks and partitions
kv net          # Network interfaces
kv cpu          # CPU info
kv mem          # Memory stats
kv mounts       # Mount points
kv thermal      # Temperature sensors
kv power        # Power supplies / batteries
kv dt           # Device tree (ARM/RISC-V)
kv snapshot     # Everything as JSON
```

### Output Formats

```bash
kv mem          # Text: KEY=VALUE pairs
kv mem -j       # JSON
kv mem -jp      # Pretty JSON
kv mem -v       # Verbose (more fields)
kv mem -h       # Human-readable sizes (16G not 16324656)
kv mem -jpvh    # Combine flags
```

### Filtering

```bash
kv block -f nvme       # Only NVMe devices
kv pci -f nvidia       # Only NVIDIA PCI devices
kv dt -f gpu           # Device tree nodes matching "gpu"
kv net -jv -f eth      # Combine with other flags (keep -f last)
```

Note: `-f` takes an argument, so keep it separate from combined flags (use `-jv -f pattern`, not `-jvf pattern`).

## Building from Source

Requires Rust 1.85+ (uses Rust 2024 edition).

> [!IMPORTANT]
> See [CONTRIBUTING.md](CONTRIBUTING.md) for full cross-compilation setup.

```bash
# Debug build
cargo build

# Release build (static, recommended)
cargo build --release --target x86_64-unknown-linux-musl
```

### Cross-Compilation

Cargo aliases for embedded targets:

```bash
cargo x86_64   # x86_64-unknown-linux-musl
cargo x86      # i686-unknown-linux-musl (32-bit)
cargo arm64    # aarch64-unknown-linux-musl
cargo aarch64  # same as arm64
cargo arm      # arm-unknown-linux-musleabihf (32-bit ARM)
cargo riscv64  # riscv64gc-unknown-linux-musl
cargo ppc      # powerpc-unknown-linux-gnu (big-endian)
```

ARM, RISC-V, and PowerPC builds automatically include the `dt` (device tree) feature.

Prerequisites (Debian/Ubuntu):

```bash
rustup target add aarch64-unknown-linux-musl
sudo apt install gcc-aarch64-linux-gnu
```

### Minimal Builds

Don't need USB? Don't compile it:

```bash
cargo build --release --no-default-features --features "mem,cpu,block"
```

## Features

| Feature | Description |
|---------|-------------|
| pci | PCI device enumeration |
| usb | USB device enumeration |
| block | Block devices and partitions |
| net | Network interfaces, IPs, wireless signal |
| cpu | CPU topology and info |
| mem | Memory information |
| mounts | Mount points |
| thermal | Temperature sensors and cooling devices |
| power | Power supplies and batteries |
| dt | Device tree (ARM/RISC-V) |
| snapshot | Combined JSON dump |

## Example Output

```bash
$ kv mem -h
MEM_TOTAL=16G MEM_FREE=121M MEM_AVAILABLE=12G SWAP_TOTAL=2G SWAP_FREE=2G

$ kv pci
BDF=0000:01:00.0 VENDOR_ID=0x10de DEVICE_ID=0x1b80 CLASS=0x030000 DRIVER=nouveau

$ kv net
NAME=eth0 MAC=dc:a6:32:56:76:50 MTU=1500 STATE=up SPEED_MBPS=1000 IP=192.168.1.100
NAME=wlan0 MAC=dc:a6:32:56:76:51 MTU=1500 STATE=up IP=192.168.1.101 SIGNAL=-52dBm

$ kv thermal -h
SENSOR=cpu-thermal TEMP=44.5°C
SENSOR=gpu-thermal TEMP=41.2°C

$ kv block -h
NAME=mmcblk0 TYPE=disk SIZE=16G MODEL="SC16G"
NAME=mmcblk0p1 TYPE=part SIZE=512M PARENT=mmcblk0 MOUNTPOINT="/boot"
NAME=mmcblk0p2 TYPE=part SIZE=15G PARENT=mmcblk0 MOUNTPOINT="/"
```

## Design Philosophy

- **No dependencies.** Zero external crates. std only.
- **Single static binary.** Copy it anywhere, it just works.
- **Read-only.** We observe, we don't touch.
- **Graceful degradation.** Missing /sys/bus/pci? We say so and move on.
- **Stable output.** Scripts can depend on the format.

## Contributing

**We want kv to work on every Linux system.** Help us by testing on your hardware!

If you have unusual embedded boards, custom SoCs, or systems where kv
doesn't work correctly, we'd love to hear from you.

**Ways to contribute:**
- [Open an issue](../../issues/new) - Bug reports, test results from your hardware
- [Start a discussion](../../discussions) - Feature requests, questions, ideas
- [Submit a pull request](../../pulls) - Code contributions

New to open source? No problem! See [CONTRIBUTING.md](CONTRIBUTING.md) for
step-by-step instructions on how to submit test results or report issues.

## Supported Architectures

| Target | Alias | Notes |
|--------|-------|-------|
| x86_64-unknown-linux-musl | `cargo x86_64` | 64-bit x86 |
| i686-unknown-linux-musl | `cargo x86` | 32-bit x86 |
| aarch64-unknown-linux-musl | `cargo arm64` / `cargo aarch64` | 64-bit ARM |
| arm-unknown-linux-musleabihf | `cargo arm` | 32-bit ARM, hard float |
| riscv64gc-unknown-linux-musl | `cargo riscv64` | 64-bit RISC-V |
| powerpc-unknown-linux-gnu | `cargo ppc` | 32-bit PowerPC (big-endian)* |

*PowerPC uses GNU libc instead of musl because `powerpc-unknown-linux-musl` isn't available
in stable Rust. The binary is still fully static but larger (~1.2 MB vs ~550 KB for musl targets).

## Security

kv is designed for untrusted environments. See the security table below.

| Protection | Implementation |
|------------|----------------|
| Memory safety | Rust's type system prevents buffer overflows, use-after-free |
| JSON escaping | Control chars, quotes, backslashes properly escaped |
| Path traversal | Rejects `..` in devicetree paths |
| Input limits | Filter patterns truncated to 1024 chars |
| Recursion limits | Devicetree traversal stops at 64 levels |
| Safe parsing | Returns `None` on overflow instead of panicking |
| Read-only | Only reads from /sys and /proc, never writes |
| No shell | No command execution, no injection surface |
| No network | Pure local filesystem operations |
| Signed releases | Cosign keyless signatures via GitHub Actions OIDC |

## License

MIT

## Author

Leon Vak <leonvak@gmail.com>
