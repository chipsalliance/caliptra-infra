# Caliptra FPGA RTL & Bitstream Builds

This directory consolidates all Caliptra FPGA wrapper configurations, Vivado build scripts, and pinned RTL submodules into a single flat layout organized by hardware release (`1.x`, `2.0`, `2.1`, `2.2`).

> **Legacy note:** [`hw/fpga/`](fpga/README.md) is retained only for `.github/workflows/fpga-image.yml` (`create_boot_bin.sh` / PetaLinux `BOOT.BIN` builds).

## Target Directories & Submodules

| Target | Board / Part | Source Repo / Branch | RTL / Subsystem Submodule Path(s) & Pinned Commits |
|---|---|---|---|
| `hw/1.x/` | ZCU104 (`xczu7ev-ffvc1156-2-e`) | `chipsalliance/caliptra-sw` (`caliptra-1.x`) | `hw/1.x/1.0/rtl` -> `caliptra-rtl` @ `7c953512251707d6f77d7d9a2f6be186ed013d50` (`patch_v1.0`)<br>`hw/1.x/latest/rtl` -> `caliptra-rtl` @ `5f85fb4bc95b753a2f7d042db7dc2644ca1e8c49` (`main` / `v1.1`) |
| `hw/2.0/core/` | VCK190 (`xcvc1902-vsva2197-2MP-e-S`) | `chipsalliance/caliptra-sw` (`caliptra-2.0`) | `hw/2.0/caliptra-ss` -> `caliptra-ss` @ `b5718a0f7fe7936b5082a8dfc23add896755af3a` (shared with `hw/2.0/subsystem/`) |
| `hw/2.0/subsystem/` | VCK190 (`xcvc1902-vsva2197-2MP-e-S`) | `chipsalliance/caliptra-mcu-sw` (`main`) | `hw/2.0/caliptra-ss` -> `caliptra-ss` @ `b5718a0f7fe7936b5082a8dfc23add896755af3a` |
| `hw/2.1/core/` | VCK190 (`xcvc1902-vsva2197-2MP-e-S`) | `chipsalliance/caliptra-sw` (`main`) | `hw/2.1/caliptra-ss` -> `caliptra-ss` @ `753308ddc82409516dd5d4069c124bba017e5ee3` (shared with `hw/2.1/subsystem/`) |
| `hw/2.1/subsystem/` | VCK190 (`xcvc1902-vsva2197-2MP-e-S`) | `chipsalliance/caliptra-mcu-sw` (`caliptra-mcu-dev-2.2:hw/rev-2_1`) | `hw/2.1/caliptra-ss` -> `caliptra-ss` @ `753308ddc82409516dd5d4069c124bba017e5ee3` |
| `hw/2.2/core/` | VCK190 (`xcvc1902-vsva2197-2MP-e-S`) | `chipsalliance/caliptra-sw` (`caliptra-2.2-dev`) | `hw/2.2/caliptra-ss` -> `caliptra-ss` @ `743d5f7eaa9a946f405b9087c7ffd2fbb38e1543` (`main`, shared with `hw/2.2/subsystem/`) |
| `hw/2.2/subsystem/` | VCK190 (`xcvc1902-vsva2197-2MP-e-S`) | `chipsalliance/caliptra-mcu-sw` (`caliptra-mcu-dev-2.2:hw/latest`) | `hw/2.2/caliptra-ss` -> `caliptra-ss` @ `743d5f7eaa9a946f405b9087c7ffd2fbb38e1543` (`main`) |

## Bitstream Variants

| Variant | Release | Subtarget | Board | Vivado Outputs (`caliptra_build/`) | GCS Publish Path |
|---|---|---|---|---|---|
| `core-hw1.0-itrng` | `1.x` | `1.x` | ZCU104 | `caliptra_fpga.bin` | `v1/<sha>/core-hw1.0-itrng/manifest.toml` |
| `core-hw1.0-etrng` | `1.x` | `1.x` | ZCU104 | `caliptra_fpga.bin` | `v1/<sha>/core-hw1.0-etrng/manifest.toml` |
| `core-latest-itrng` | `1.x` | `1.x` | ZCU104 | `caliptra_fpga.bin` | `v1/<sha>/core-latest-itrng/manifest.toml` |
| `core-latest-etrng` | `1.x` | `1.x` | ZCU104 | `caliptra_fpga.bin` | `v1/<sha>/core-latest-etrng/manifest.toml` |
| `core-2.0-segmented` | `2.0` | `2.0/core` | VCK190 | `caliptra_fpga.xsa`, `runtime_<ver>.pdi` | `v1/<sha>/core-2.0-segmented/manifest.toml` |
| `core-2.0-standard` | `2.0` | `2.0/core` | VCK190 | `caliptra_fpga.xsa` | *(artifact only)* |
| `subsystem-2.0-segmented` | `2.0` | `2.0/subsystem` | VCK190 | `xsa_ss2p0_<ver>_segmented.xsa`, `runtime_<ver>.pdi` | `v1/<sha>/subsystem-2.0-segmented/manifest.toml` |
| `subsystem-2.0-standard` | `2.0` | `2.0/subsystem` | VCK190 | `xsa_ss2p0_<ver>.xsa` | *(artifact only)* |
| `core-2.1-segmented` | `2.1` | `2.1/core` | VCK190 | `caliptra_fpga.xsa`, `runtime_<ver>.pdi` | `v1/<sha>/core-2.1-segmented/manifest.toml` |
| `core-2.1-standard` | `2.1` | `2.1/core` | VCK190 | `caliptra_fpga.xsa` | *(artifact only)* |
| `subsystem-2.1-segmented` | `2.1` | `2.1/subsystem` | VCK190 | `xsa_ss2p1_<ver>_segmented.xsa`, `runtime_<ver>.pdi` | `v1/<sha>/subsystem-2.1-segmented/manifest.toml` |
| `subsystem-2.1-standard` | `2.1` | `2.1/subsystem` | VCK190 | `xsa_ss2p1_<ver>.xsa` | *(artifact only)* |
| `core-2.2-segmented` | `2.2` | `2.2/core` | VCK190 | `caliptra_fpga.xsa`, `runtime_<ver>.pdi` | `v1/<sha>/core-2.2-segmented/manifest.toml` |
| `core-2.2-standard` | `2.2` | `2.2/core` | VCK190 | `caliptra_fpga.xsa` | *(artifact only)* |
| `subsystem-2.2-segmented` | `2.2` | `2.2/subsystem` | VCK190 | `xsa_ss2p2_<ver>_segmented.xsa`, `runtime_<ver>.pdi` | `v1/<sha>/subsystem-2.2-segmented/manifest.toml` |
| `subsystem-2.2-standard` | `2.2` | `2.2/subsystem` | VCK190 | `xsa_ss2p2_<ver>.xsa` | *(artifact only)* |

## Usage

### Local Build (`caliptra-bitstream-downloader`)

All FPGA variant metadata, submodule initialization, Vivado invocations, CI matrix calculation, and bitstream manifest bundling are implemented in the [`caliptra-bitstream-downloader`](../ci-tools/bitstream-downloader) Rust CLI.

```bash
# List all 16 variants, release folders, and subtargets
cargo run --manifest-path ci-tools/Cargo.toml -p caliptra-bitstream-downloader -- list

# Preview commands without invoking Vivado or initializing submodules
cargo run --manifest-path ci-tools/Cargo.toml -p caliptra-bitstream-downloader -- build --dry-run 2.2/subsystem

# Build a single variant (initializes only the needed submodule and runs Vivado 2024.2)
cargo run --manifest-path ci-tools/Cargo.toml -p caliptra-bitstream-downloader -- build --staging-dir /tmp/bitstream subsystem-2.2-segmented

# Build a subtarget (core or subsystem) or an entire release folder
cargo run --manifest-path ci-tools/Cargo.toml -p caliptra-bitstream-downloader -- build 2.1/core
cargo run --manifest-path ci-tools/Cargo.toml -p caliptra-bitstream-downloader -- build 2.0
cargo run --manifest-path ci-tools/Cargo.toml -p caliptra-bitstream-downloader -- build all

# CI helper subcommands (invoked by .github/workflows/bitstream-build.yml)
cargo run --manifest-path ci-tools/Cargo.toml -p caliptra-bitstream-downloader -- ci-matrix
cargo run --manifest-path ci-tools/Cargo.toml -p caliptra-bitstream-downloader -- bundle --variant subsystem-2.2-segmented --staging-dir /tmp/bitstream --output-dir manifest-bundle
```

### CI Workflow (`.github/workflows/bitstream-build.yml`)

- Runs `caliptra-bitstream-downloader` via `cargo run --manifest-path ci-tools/Cargo.toml -p caliptra-bitstream-downloader -- ...` (using `dtolnay/rust-toolchain@1.85.1` on `[bitstream-builder]`) to compute the matrix (`ci-matrix`), initialize submodules and run Vivado (`build`), and package/summarize bundles (`bundle`).

- **On `push` to `main`**: Detects granular subtarget changes (`hw/1.x/**`, `hw/2.{0,1,2}/{core,subsystem}/**`) as well as shared 2.x submodule updates (`hw/2.{0,1,2}/caliptra-ss`, which trigger both `{core,subsystem}` for that release), or builds `all` when `.github/workflows/bitstream-build.yml` or `ci-tools/bitstream-downloader/**` changes.
- **On `workflow_dispatch`**: Choose a `target` (`all`, `1.x`, `2.0`, `2.0/core`, `2.0/subsystem`, `2.1`, `2.1/core`, `2.1/subsystem`, `2.2`, `2.2/core`, `2.2/subsystem`) or an explicit `variant`, and toggle `include_standard` to include/exclude non-segmented VCK190 builds.
- **Publishing**: Segmented VCK190 variants and all ZCU104 1.x variants leave `manifest-bundle/caliptra-bitstream.tar.gz` for the `bitstream-builder` runner's post-job hook to upload to `gs://caliptra-github-ci-bitstreams/v1/<sha>/<variant>/`, and write the HTTPS manifest URL (`https://storage.googleapis.com/caliptra-github-ci-bitstreams/v1/<sha>/<variant>/manifest.toml`) to the workflow step summary.
