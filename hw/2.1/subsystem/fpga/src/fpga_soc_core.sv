// Licensed under the Apache-2.0 license

// Small RV32IMC core (PicoRV32) with its own SRAM and a pair of byte FIFOs.
// It stands in for an SoC component whose firmware is loaded by the MCU and
// released from reset by Caliptra.
//
// Memory map seen by the core:
//   0x0000_0000  SRAM (SRAM_BYTES). Execution starts at 0x0000_0000.
//   0x1000_0000  DATA_OUT  write: push bits [7:0] to the host. Dropped if full.
//   0x1000_0004  DATA_IN   read: [8] valid, [7:0] next byte from the host.
//                          Reading removes the byte.
//   0x1000_0008  STATUS    read: [0] DATA_OUT full, [1] DATA_OUT empty,
//                          [2] DATA_IN empty, [3] DATA_IN full.
// Other addresses read as zero and ignore writes.
//
// The SRAM is also reachable through the mem_* port so that firmware can be
// loaded over the AXI bus. It powers up as zero and is kept across resets.
module fpga_soc_core #(
    // SRAM size in bytes. Must be a power of two.
    parameter int SRAM_BYTES = 8 * 1024,
    parameter int TO_HOST_FIFO_DEPTH = 1024,
    parameter int TO_CORE_FIFO_DEPTH = 64,
    localparam int SRAM_AW = $clog2(SRAM_BYTES)
) (
    input  logic        clk,
    // Synchronous, active high. Empties the FIFOs.
    input  logic        rst,
    // Core reset, active low. Also empties the FIFO to the core.
    input  logic        core_rst_b,

    // The core stopped on an illegal instruction, misaligned access or ebreak.
    output logic        trap,

    // SRAM access from outside the core.
    // mem_addr is a byte address; bits [1:0] are ignored.
    // mem_rdata is valid in the cycle after mem_en.
    input  logic               mem_en,
    input  logic [3:0]         mem_we,
    input  logic [SRAM_AW-1:0] mem_addr,
    input  logic [31:0]        mem_wdata,
    output logic [31:0]        mem_rdata,

    // Host side of the FIFOs
    input  logic        to_host_pop,
    output logic [7:0]  to_host_data,
    output logic        to_host_empty,
    output logic        to_host_full,
    input  logic        to_core_push,
    input  logic [7:0]  to_core_data,
    output logic        to_core_empty,
    output logic        to_core_full
);
    localparam logic [3:0] MMIO_REGION = 4'h1;

    logic        cpu_mem_valid;
    logic        cpu_mem_ready;
    logic [31:0] cpu_mem_addr;
    logic [31:0] cpu_mem_wdata;
    logic [3:0]  cpu_mem_wstrb;
    logic [31:0] cpu_mem_rdata;

    picorv32 #(
        .ENABLE_COUNTERS(0),
        .ENABLE_COUNTERS64(0),
        .ENABLE_REGS_16_31(1),
        .ENABLE_REGS_DUALPORT(1),
        .LATCHED_MEM_RDATA(0),
        .TWO_STAGE_SHIFT(1),
        .BARREL_SHIFTER(0),
        .TWO_CYCLE_COMPARE(0),
        .TWO_CYCLE_ALU(0),
        .COMPRESSED_ISA(1),
        .CATCH_MISALIGN(1),
        .CATCH_ILLINSN(1),
        .ENABLE_PCPI(0),
        .ENABLE_MUL(1),
        .ENABLE_FAST_MUL(0),
        .ENABLE_DIV(1),
        .ENABLE_IRQ(0),
        .ENABLE_TRACE(0),
        .REGS_INIT_ZERO(0),
        .PROGADDR_RESET(32'h0000_0000),
        .STACKADDR(32'hffff_ffff)
    ) cpu (
        .clk(clk),
        .resetn(core_rst_b),
        .trap(trap),

        .mem_valid(cpu_mem_valid),
        .mem_instr(),
        .mem_ready(cpu_mem_ready),
        .mem_addr(cpu_mem_addr),
        .mem_wdata(cpu_mem_wdata),
        .mem_wstrb(cpu_mem_wstrb),
        .mem_rdata(cpu_mem_rdata),

        .mem_la_read(),
        .mem_la_write(),
        .mem_la_addr(),
        .mem_la_wdata(),
        .mem_la_wstrb(),

        .pcpi_valid(),
        .pcpi_insn(),
        .pcpi_rs1(),
        .pcpi_rs2(),
        .pcpi_wr(1'b0),
        .pcpi_rd(32'b0),
        .pcpi_wait(1'b0),
        .pcpi_ready(1'b0),

        .irq(32'b0),
        .eoi(),

        .trace_valid(),
        .trace_data()
    );

    // Every access takes two cycles: the request is issued in the first and
    // cpu_mem_ready is high in the second.
    logic cpu_req;
    logic sel_sram;
    logic sel_mmio;
    assign cpu_req  = cpu_mem_valid && !cpu_mem_ready;
    assign sel_sram = (cpu_mem_addr[31:SRAM_AW] == '0);
    assign sel_mmio = (cpu_mem_addr[31:28] == MMIO_REGION);

    logic        sel_sram_f;
    logic [31:0] mmio_rdata_f;
    logic [31:0] sram_rdata;

    logic       to_host_push;
    logic       to_core_pop;
    logic [7:0] to_core_head;

    assign to_host_push = cpu_req && sel_mmio && (cpu_mem_addr[7:2] == 6'h0) && cpu_mem_wstrb[0];
    assign to_core_pop  = cpu_req && sel_mmio && (cpu_mem_addr[7:2] == 6'h1) && (cpu_mem_wstrb == '0);

    always_ff @(posedge clk) begin
        if (!core_rst_b) begin
            cpu_mem_ready <= 1'b0;
            sel_sram_f    <= 1'b0;
            mmio_rdata_f  <= '0;
        end else begin
            cpu_mem_ready <= cpu_req;
            sel_sram_f    <= sel_sram;
            mmio_rdata_f  <= '0;
            if (sel_mmio) begin
                unique case (cpu_mem_addr[7:2])
                    6'h1:    mmio_rdata_f <= {23'b0, !to_core_empty, to_core_head};
                    6'h2:    mmio_rdata_f <= {28'b0, to_core_full, to_core_empty, to_host_empty, to_host_full};
                    default: mmio_rdata_f <= '0;
                endcase
            end
        end
    end

    assign cpu_mem_rdata = sel_sram_f ? sram_rdata : mmio_rdata_f;

    fpga_sync_fifo #(
        .WIDTH(8),
        .DEPTH(TO_HOST_FIFO_DEPTH)
    ) to_host_fifo (
        .clk(clk),
        .rst(rst),
        .wr_en(to_host_push),
        .din(cpu_mem_wdata[7:0]),
        .full(to_host_full),
        .rd_en(to_host_pop),
        .dout(to_host_data),
        .empty(to_host_empty)
    );

    fpga_sync_fifo #(
        .WIDTH(8),
        .DEPTH(TO_CORE_FIFO_DEPTH)
    ) to_core_fifo (
        .clk(clk),
        .rst(rst || !core_rst_b),
        .wr_en(to_core_push),
        .din(to_core_data),
        .full(to_core_full),
        .rd_en(to_core_pop),
        .dout(to_core_head),
        .empty(to_core_empty)
    );

    // Port A is the core, port B is the outside access port.
    xpm_memory_tdpram #(
        .ADDR_WIDTH_A(SRAM_AW - 2),
        .ADDR_WIDTH_B(SRAM_AW - 2),
        .AUTO_SLEEP_TIME(0),
        .BYTE_WRITE_WIDTH_A(8),
        .BYTE_WRITE_WIDTH_B(8),
        .CASCADE_HEIGHT(0),
        .CLOCKING_MODE("common_clock"),
        .ECC_MODE("no_ecc"),
        .MEMORY_INIT_FILE("none"),
        .MEMORY_INIT_PARAM("0"),
        .MEMORY_OPTIMIZATION("false"),
        .MEMORY_PRIMITIVE("block"),
        .MEMORY_SIZE(SRAM_BYTES * 8),
        .MESSAGE_CONTROL(0),
        .READ_DATA_WIDTH_A(32),
        .READ_DATA_WIDTH_B(32),
        .READ_LATENCY_A(1),
        .READ_LATENCY_B(1),
        .READ_RESET_VALUE_A("0"),
        .READ_RESET_VALUE_B("0"),
        .RST_MODE_A("SYNC"),
        .RST_MODE_B("SYNC"),
        .SIM_ASSERT_CHK(0),
        .USE_EMBEDDED_CONSTRAINT(0),
        .USE_MEM_INIT(1),
        .USE_MEM_INIT_MMI(0),
        .WAKEUP_TIME("disable_sleep"),
        .WRITE_DATA_WIDTH_A(32),
        .WRITE_DATA_WIDTH_B(32),
        .WRITE_MODE_A("no_change"),
        .WRITE_MODE_B("no_change"),
        .WRITE_PROTECT(1)
    ) sram (
        .dbiterra(),
        .dbiterrb(),
        .douta(sram_rdata),
        .doutb(mem_rdata),
        .sbiterra(),
        .sbiterrb(),
        .addra(cpu_mem_addr[SRAM_AW-1:2]),
        .addrb(mem_addr[SRAM_AW-1:2]),
        .clka(clk),
        .clkb(clk),
        .dina(cpu_mem_wdata),
        .dinb(mem_wdata),
        .ena(cpu_req && sel_sram),
        .enb(mem_en),
        .injectdbiterra(1'b0),
        .injectdbiterrb(1'b0),
        .injectsbiterra(1'b0),
        .injectsbiterrb(1'b0),
        .regcea(1'b1),
        .regceb(1'b1),
        .rsta(1'b0),
        .rstb(1'b0),
        .sleep(1'b0),
        .wea(cpu_mem_wstrb),
        .web(mem_we)
    );
endmodule
