#!/bin/bash
# Licensed under the Apache-2.0 license

# Simulates fpga_fw_update_devices_tb with the Vivado simulator.
# Usage: run_sim.sh [work directory]
# Needs xvlog, xelab and xsim on PATH or XILINX_VIVADO set, and cargo to build
# the SoC core firmware.

set -euo pipefail

tb_dir=$(cd "$(dirname "$0")" && pwd)
fpga_dir=$(dirname "$tb_dir")
rtl_dir=$fpga_dir/../../caliptra-ss/third_party/caliptra-rtl/src
work_dir=${1:-$tb_dir/sim_build}

if [[ -z "${XILINX_VIVADO:-}" ]]; then
    XILINX_VIVADO=$(dirname "$(dirname "$(readlink -f "$(command -v xvlog)")")")
fi
PATH=$XILINX_VIVADO/bin:$PATH

mkdir -p "$work_dir"
"$tb_dir/soc_core_fw/build.sh" "$work_dir"

cd "$work_dir"
xvlog -sv \
    -i "$rtl_dir/caliptra_prim/rtl" \
    -i "$rtl_dir/libs/rtl" \
    "$XILINX_VIVADO/data/ip/xpm/xpm_memory/hdl/xpm_memory.sv" \
    "$rtl_dir/axi/rtl/axi_pkg.sv" \
    "$rtl_dir/axi/rtl/axi_if.sv" \
    "$rtl_dir/axi/rtl/axi_addr.v" \
    "$rtl_dir/libs/rtl/skidbuffer.v" \
    "$rtl_dir/axi/rtl/axi_sub_rd.sv" \
    "$rtl_dir/axi/rtl/axi_sub_wr.sv" \
    "$rtl_dir/axi/rtl/axi_sub_arb.sv" \
    "$rtl_dir/axi/rtl/axi_sub.sv" \
    "$fpga_dir/third_party/picorv32/picorv32.v" \
    "$fpga_dir/src/axi4lite_intf.sv" \
    "$fpga_dir/src/caliptra_fpga_realtime_regs_pkg.sv" \
    "$fpga_dir/src/caliptra_fpga_realtime_regs.sv" \
    "$fpga_dir/src/fpga_sync_fifo.sv" \
    "$fpga_dir/src/fpga_hw_flash.sv" \
    "$fpga_dir/src/fpga_soc_core.sv" \
    "$fpga_dir/src/fpga_fw_update_devices.sv" \
    "$tb_dir/fpga_fw_update_devices_tb.sv"
xvlog "$XILINX_VIVADO/data/verilog/src/glbl.v"
xelab -debug off fpga_fw_update_devices_tb glbl -s fpga_fw_update_devices_tb_sim
xsim fpga_fw_update_devices_tb_sim -R -testplusarg "SOC_CORE_FW=$work_dir/soc_core_fw.hex" | tee sim.log
grep -q "^TEST PASSED" sim.log
