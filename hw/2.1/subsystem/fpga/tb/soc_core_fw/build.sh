#!/bin/bash
# Licensed under the Apache-2.0 license

# Builds the SoC core test firmware.
# Usage: build.sh <output directory>
# Writes soc_core_fw.bin and soc_core_fw.hex (one 32 bit little-endian word
# per line, for $readmemh). Needs cargo; rust-toolchain.toml selects the
# toolchain and the riscv32imc-unknown-none-elf target.

set -euo pipefail

src_dir=$(cd "$(dirname "$0")" && pwd)
out_dir=${1:?usage: build.sh <output directory>}
target=riscv32imc-unknown-none-elf

mkdir -p "$out_dir"
out_dir=$(cd "$out_dir" && pwd)
(cd "$src_dir" && cargo build --release --locked --target "$target" --target-dir "$out_dir/target")
cp "$out_dir/target/$target/release/soc_core_fw" "$out_dir/soc_core_fw.bin"
od -An -v -tx4 -w4 "$out_dir/soc_core_fw.bin" | tr -d ' ' > "$out_dir/soc_core_fw.hex"
