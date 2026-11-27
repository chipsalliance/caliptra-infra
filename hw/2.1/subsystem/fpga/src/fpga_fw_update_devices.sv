// Licensed under the Apache-2.0 license

// Devices for exercising firmware update flows on the FPGA:
//   - two flash controllers backed by on-chip RAM (fpga_hw_flash),
//   - a small RV32 core with its own SRAM and FIFOs (fpga_soc_core), held in
//     reset until Caliptra sets the selected SS_GENERIC_FW_EXEC_CTRL bit.
//
// The flash contents and the SoC core SRAM are mapped on one AXI subordinate
// port. Bits [25:24] of the address select the memory:
//   0  SoC core SRAM
//   1  primary flash
//   2  secondary flash
// Addresses past the end of a memory read as zero and ignore writes.
//
// The registers are part of caliptra_fpga_realtime_regs.
module fpga_fw_update_devices
    import caliptra_fpga_realtime_regs_pkg::*;
#(
    parameter int PRIMARY_FLASH_BYTES   = 4 * 1024 * 1024,
    parameter int SECONDARY_FLASH_BYTES = 4 * 1024 * 1024,
    parameter int SOC_SRAM_BYTES        = 8 * 1024,
    parameter int AXI_IW                = 8,
    parameter int AXI_UW                = 32
) (
    input  logic clk,
    // Synchronous, active high.
    input  logic rst,

    // AXI subordinate port for the memories
    axi_if.w_sub s_axi_w_if,
    axi_if.r_sub s_axi_r_if,

    // SS_GENERIC_FW_EXEC_CTRL from Caliptra
    input  logic [127:0] fw_exec_ctrl,

    // Register block
    input  caliptra_fpga_realtime_regs__out_t        hwif_out,
    output hw_flash_ctrl_regs__external__in_t        primary_flash_regs,
    output hw_flash_ctrl_regs__external__in_t        secondary_flash_regs,
    output fifo_regs__soc_fifo_pop__in_t             soc_fifo_pop,
    output fifo_regs__soc_fifo_status__in_t          soc_fifo_status,
    output interface_regs__soc_core_status__in_t     soc_core_status,

    output logic primary_flash_irq,
    output logic secondary_flash_irq
);
    localparam int SLOT_AW = 24;
    localparam logic [1:0] SLOT_SOC_SRAM        = 2'd0;
    localparam logic [1:0] SLOT_PRIMARY_FLASH   = 2'd1;
    localparam logic [1:0] SLOT_SECONDARY_FLASH = 2'd2;

    localparam int PRIMARY_AW   = $clog2(PRIMARY_FLASH_BYTES);
    localparam int SECONDARY_AW = $clog2(SECONDARY_FLASH_BYTES);
    localparam int SOC_SRAM_AW  = $clog2(SOC_SRAM_BYTES);

    // AXI to memory request. Read data is due one cycle after dv.
    logic        dv;
    logic [31:0] addr;
    logic        write;
    logic [31:0] wdata;
    logic [3:0]  wstrb;
    logic [31:0] rdata;

    axi_sub #(
        .AW   (32),
        .DW   (32),
        .UW   (AXI_UW),
        .IW   (AXI_IW),
        .EX_EN(0),
        .C_LAT(1)
    ) mem_axi_sub (
        .clk  (clk),
        .rst_n(!rst),

        .s_axi_w_if(s_axi_w_if),
        .s_axi_r_if(s_axi_r_if),

        .dv    (dv),
        .addr  (addr),
        .write (write),
        .user  (),
        .id    (),
        .wdata (wdata),
        .wstrb (wstrb),
        .size  (),
        .rdata (rdata),
        .last  (),
        .hld   (1'b0),
        .rd_err(1'b0),
        .wr_err(1'b0)
    );

    logic [1:0] slot;
    logic       sel_soc_sram;
    logic       sel_primary;
    logic       sel_secondary;
    assign slot          = addr[SLOT_AW+1:SLOT_AW];
    assign sel_soc_sram  = dv && (slot == SLOT_SOC_SRAM)        && (addr[SLOT_AW-1:SOC_SRAM_AW]  == '0);
    assign sel_primary   = dv && (slot == SLOT_PRIMARY_FLASH)   && (addr[SLOT_AW-1:PRIMARY_AW]   == '0);
    assign sel_secondary = dv && (slot == SLOT_SECONDARY_FLASH) && (addr[SLOT_AW-1:SECONDARY_AW] == '0);

    logic [3:0] mem_we;
    assign mem_we = write ? wstrb : 4'b0;

    logic [31:0] soc_sram_rdata;
    logic [31:0] primary_rdata;
    logic [31:0] secondary_rdata;
    logic        sel_soc_sram_f;
    logic        sel_primary_f;
    logic        sel_secondary_f;

    always_ff @(posedge clk) begin
        if (rst) begin
            sel_soc_sram_f  <= 1'b0;
            sel_primary_f   <= 1'b0;
            sel_secondary_f <= 1'b0;
        end else begin
            sel_soc_sram_f  <= sel_soc_sram;
            sel_primary_f   <= sel_primary;
            sel_secondary_f <= sel_secondary;
        end
    end

    always_comb begin
        rdata = '0;
        if (sel_soc_sram_f)  rdata = soc_sram_rdata;
        if (sel_primary_f)   rdata = primary_rdata;
        if (sel_secondary_f) rdata = secondary_rdata;
    end

    // Flash controllers
    fpga_hw_flash #(
        .SIZE_BYTES(PRIMARY_FLASH_BYTES)
    ) primary_flash (
        .clk      (clk),
        .rst      (rst),
        .req      (hwif_out.primary_hw_flash_ctrl_regs.req),
        .req_is_wr(hwif_out.primary_hw_flash_ctrl_regs.req_is_wr),
        .addr     (hwif_out.primary_hw_flash_ctrl_regs.addr),
        .wr_data  (hwif_out.primary_hw_flash_ctrl_regs.wr_data),
        .wr_biten (hwif_out.primary_hw_flash_ctrl_regs.wr_biten),
        .rd_ack   (primary_flash_regs.rd_ack),
        .rd_data  (primary_flash_regs.rd_data),
        .wr_ack   (primary_flash_regs.wr_ack),
        .mem_en   (sel_primary),
        .mem_we   (mem_we),
        .mem_addr (addr[PRIMARY_AW-1:0]),
        .mem_wdata(wdata),
        .mem_rdata(primary_rdata),
        .irq      (primary_flash_irq)
    );

    fpga_hw_flash #(
        .SIZE_BYTES(SECONDARY_FLASH_BYTES)
    ) secondary_flash (
        .clk      (clk),
        .rst      (rst),
        .req      (hwif_out.secondary_hw_flash_ctrl_regs.req),
        .req_is_wr(hwif_out.secondary_hw_flash_ctrl_regs.req_is_wr),
        .addr     (hwif_out.secondary_hw_flash_ctrl_regs.addr),
        .wr_data  (hwif_out.secondary_hw_flash_ctrl_regs.wr_data),
        .wr_biten (hwif_out.secondary_hw_flash_ctrl_regs.wr_biten),
        .rd_ack   (secondary_flash_regs.rd_ack),
        .rd_data  (secondary_flash_regs.rd_data),
        .wr_ack   (secondary_flash_regs.wr_ack),
        .mem_en   (sel_secondary),
        .mem_we   (mem_we),
        .mem_addr (addr[SECONDARY_AW-1:0]),
        .mem_wdata(wdata),
        .mem_rdata(secondary_rdata),
        .irq      (secondary_flash_irq)
    );

    // SoC core reset: released by the selected SS_GENERIC_FW_EXEC_CTRL bit.
    logic soc_core_rst_b;
    always_ff @(posedge clk) begin
        if (rst) begin
            soc_core_rst_b <= 1'b0;
        end else begin
            soc_core_rst_b <= !hwif_out.interface_regs.soc_core_control.force_reset.value &&
                              (hwif_out.interface_regs.soc_core_control.force_run.value ||
                               fw_exec_ctrl[hwif_out.interface_regs.soc_core_control.exec_bit.value]);
        end
    end

    logic soc_core_trap;
    assign soc_core_status.rst_b.next = soc_core_rst_b;
    assign soc_core_status.trap.next  = soc_core_trap;

    // The written value reaches hwif_out one cycle after wr_swacc.
    logic to_core_push;
    always_ff @(posedge clk) begin
        if (rst) begin
            to_core_push <= 1'b0;
        end else begin
            to_core_push <= hwif_out.fifo_regs.soc_fifo_push.in_data.wr_swacc;
        end
    end

    logic       to_host_empty;
    logic [7:0] to_host_data;
    assign soc_fifo_pop.valid.next    = !to_host_empty;
    assign soc_fifo_pop.out_data.next = to_host_empty ? 8'h0 : to_host_data;
    assign soc_fifo_status.to_host_empty.next = to_host_empty;

    fpga_soc_core #(
        .SRAM_BYTES(SOC_SRAM_BYTES)
    ) soc_core (
        .clk          (clk),
        .rst          (rst),
        .core_rst_b   (soc_core_rst_b),
        .trap         (soc_core_trap),
        .mem_en       (sel_soc_sram),
        .mem_we       (mem_we),
        .mem_addr     (addr[SOC_SRAM_AW-1:0]),
        .mem_wdata    (wdata),
        .mem_rdata    (soc_sram_rdata),
        .to_host_pop  (hwif_out.fifo_regs.soc_fifo_pop.out_data.rd_swacc),
        .to_host_data (to_host_data),
        .to_host_empty(to_host_empty),
        .to_host_full (soc_fifo_status.to_host_full.next),
        .to_core_push (to_core_push),
        .to_core_data (hwif_out.fifo_regs.soc_fifo_push.in_data.value),
        .to_core_empty(soc_fifo_status.to_core_empty.next),
        .to_core_full (soc_fifo_status.to_core_full.next)
    );
endmodule
