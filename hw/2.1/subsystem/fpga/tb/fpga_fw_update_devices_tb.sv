// Licensed under the Apache-2.0 license

// Testbench for fpga_fw_update_devices and the registers it uses in
// caliptra_fpga_realtime_regs. Run with run_sim.sh.

`timescale 1ns / 1ps

module fpga_fw_update_devices_tb;
    import caliptra_fpga_realtime_regs_pkg::*;

    localparam logic [31:0] REGS_BASE            = 32'hA401_0000;
    localparam logic [31:0] FPGA_MAGIC           = REGS_BASE + 32'h0000;
    localparam logic [31:0] FPGA_FEATURES        = REGS_BASE + 32'h0300;
    localparam logic [31:0] SOC_CORE_CONTROL     = REGS_BASE + 32'h0304;
    localparam logic [31:0] SOC_CORE_STATUS      = REGS_BASE + 32'h0308;
    localparam logic [31:0] CDMA_AXI_USER        = REGS_BASE + 32'h030C;
    localparam logic [31:0] FW_EXEC_CTRL         = REGS_BASE + 32'h0310;
    localparam logic [31:0] SOC_FIFO_POP         = REGS_BASE + 32'h1028;
    localparam logic [31:0] SOC_FIFO_PUSH        = REGS_BASE + 32'h102C;
    localparam logic [31:0] SOC_FIFO_STATUS      = REGS_BASE + 32'h1030;
    localparam logic [31:0] OLD_PRIMARY_FLASH    = REGS_BASE + 32'h2000;
    localparam logic [31:0] PRIMARY_FLASH        = REGS_BASE + 32'h4000;
    localparam logic [31:0] SECONDARY_FLASH      = REGS_BASE + 32'h5000;

    localparam logic [31:0] FL_INTERRUPT_STATE   = 32'h00;
    localparam logic [31:0] FL_INTERRUPT_ENABLE  = 32'h04;
    localparam logic [31:0] PAGE_SIZE            = 32'h08;
    localparam logic [31:0] PAGE_NUM             = 32'h0C;
    localparam logic [31:0] PAGE_ADDR            = 32'h10;
    localparam logic [31:0] FL_CONTROL           = 32'h14;
    localparam logic [31:0] OP_STATUS            = 32'h18;
    localparam logic [31:0] CTRL_REGWEN          = 32'h1C;
    localparam logic [31:0] FLASH_SIZE           = 32'h20;
    localparam logic [31:0] FLASH_BUF            = 32'h100;

    localparam logic [31:0] MEM_BASE             = 32'hB400_0000;
    localparam logic [31:0] SOC_SRAM_MEM         = MEM_BASE;
    localparam logic [31:0] PRIMARY_FLASH_MEM    = MEM_BASE + 32'h0100_0000;
    localparam logic [31:0] SECONDARY_FLASH_MEM  = MEM_BASE + 32'h0200_0000;

    localparam int FLASH_BYTES = 4 * 1024 * 1024;
    localparam int SOC_SRAM_BYTES = 8 * 1024;

    localparam int OP_READ  = 1;
    localparam int OP_WRITE = 2;
    localparam int OP_ERASE = 3;

    logic clk = 1'b0;
    logic rst = 1'b1;
    always #5 clk = ~clk;

    int errors = 0;

    // DUT
    axi4lite_intf regs_axil ();
    caliptra_fpga_realtime_regs__in_t  hwif_in;
    caliptra_fpga_realtime_regs__out_t hwif_out;

    caliptra_fpga_realtime_regs regs (
        .clk     (clk),
        .rst     (rst),
        .s_axil  (regs_axil),
        .hwif_in (hwif_in),
        .hwif_out(hwif_out)
    );

    axi_if #(
        .AW(32),
        .DW(32),
        .IW(8),
        .UW(32)
    ) mem_axi (.clk(clk), .rst_n(!rst));

    logic [127:0] fw_exec_ctrl = '0;
    logic primary_flash_irq;
    logic secondary_flash_irq;

    fpga_fw_update_devices dut (
        .clk                 (clk),
        .rst                 (rst),
        .s_axi_w_if          (mem_axi.w_sub),
        .s_axi_r_if          (mem_axi.r_sub),
        .fw_exec_ctrl        (fw_exec_ctrl),
        .hwif_out            (hwif_out),
        .primary_flash_regs  (hwif_in.primary_hw_flash_ctrl_regs),
        .secondary_flash_regs(hwif_in.secondary_hw_flash_ctrl_regs),
        .soc_fifo_pop        (hwif_in.fifo_regs.soc_fifo_pop),
        .soc_fifo_status     (hwif_in.fifo_regs.soc_fifo_status),
        .soc_core_status     (hwif_in.interface_regs.soc_core_status),
        .primary_flash_irq   (primary_flash_irq),
        .secondary_flash_irq (secondary_flash_irq)
    );

    for (genvar i = 0; i < 4; i++) begin : g_fw_exec_ctrl
        assign hwif_in.interface_regs.fw_exec_ctrl[i].value.next = fw_exec_ctrl[32*i +: 32];
    end

    // ---------------------------------------------------------------------
    // Checks
    // ---------------------------------------------------------------------
    task automatic check(input string what, input logic [31:0] actual, input logic [31:0] expected);
        if (actual !== expected) begin
            $display("FAIL: %s: got 0x%08x, expected 0x%08x (t=%0t)", what, actual, expected, $time);
            errors++;
        end
    endtask

    // ---------------------------------------------------------------------
    // AXI4-Lite manager for the registers
    // ---------------------------------------------------------------------
    initial begin
        regs_axil.AWVALID = 1'b0;
        regs_axil.AWADDR  = '0;
        regs_axil.AWPROT  = '0;
        regs_axil.WVALID  = 1'b0;
        regs_axil.WDATA   = '0;
        regs_axil.WSTRB   = '0;
        regs_axil.BREADY  = 1'b0;
        regs_axil.ARVALID = 1'b0;
        regs_axil.ARADDR  = '0;
        regs_axil.ARPROT  = '0;
        regs_axil.RREADY  = 1'b0;
    end

    task automatic reg_write_strb(input logic [31:0] addr, input logic [31:0] data, input logic [3:0] strb);
        @(posedge clk);
        regs_axil.AWVALID <= 1'b1;
        regs_axil.AWADDR  <= addr;
        regs_axil.WVALID  <= 1'b1;
        regs_axil.WDATA   <= data;
        regs_axil.WSTRB   <= strb;
        regs_axil.BREADY  <= 1'b1;
        fork
            begin
                do @(posedge clk); while (!regs_axil.AWREADY);
                regs_axil.AWVALID <= 1'b0;
            end
            begin
                do @(posedge clk); while (!regs_axil.WREADY);
                regs_axil.WVALID <= 1'b0;
            end
        join
        while (!regs_axil.BVALID) @(posedge clk);
        if (regs_axil.BRESP !== 2'b00) begin
            $display("FAIL: write to 0x%08x: BRESP %0d", addr, regs_axil.BRESP);
            errors++;
        end
        @(posedge clk);
        regs_axil.BREADY <= 1'b0;
    endtask

    task automatic reg_write(input logic [31:0] addr, input logic [31:0] data);
        reg_write_strb(addr, data, 4'hf);
    endtask

    task automatic reg_read(input logic [31:0] addr, output logic [31:0] data);
        @(posedge clk);
        regs_axil.ARVALID <= 1'b1;
        regs_axil.ARADDR  <= addr;
        regs_axil.RREADY  <= 1'b1;
        do @(posedge clk); while (!regs_axil.ARREADY);
        regs_axil.ARVALID <= 1'b0;
        while (!regs_axil.RVALID) @(posedge clk);
        data = regs_axil.RDATA[31:0];
        if (regs_axil.RRESP !== 2'b00) begin
            $display("FAIL: read of 0x%08x: RRESP %0d", addr, regs_axil.RRESP);
            errors++;
        end
        @(posedge clk);
        regs_axil.RREADY <= 1'b0;
    endtask

    task automatic reg_expect(input string what, input logic [31:0] addr, input logic [31:0] expected);
        logic [31:0] data;
        reg_read(addr, data);
        check(what, data, expected);
    endtask

    // ---------------------------------------------------------------------
    // AXI4 manager for the memory port
    // ---------------------------------------------------------------------
    initial begin
        mem_axi.awvalid = 1'b0;
        mem_axi.awaddr  = '0;
        mem_axi.awburst = 2'b01;
        mem_axi.awsize  = 3'd2;
        mem_axi.awlen   = '0;
        mem_axi.awuser  = '0;
        mem_axi.awid    = '0;
        mem_axi.awlock  = 1'b0;
        mem_axi.wvalid  = 1'b0;
        mem_axi.wdata   = '0;
        mem_axi.wstrb   = '0;
        mem_axi.wlast   = 1'b0;
        mem_axi.bready  = 1'b0;
        mem_axi.arvalid = 1'b0;
        mem_axi.araddr  = '0;
        mem_axi.arburst = 2'b01;
        mem_axi.arsize  = 3'd2;
        mem_axi.arlen   = '0;
        mem_axi.aruser  = '0;
        mem_axi.arid    = '0;
        mem_axi.arlock  = 1'b0;
        mem_axi.rready  = 1'b0;
    end

    // Incrementing burst write of data[0 .. beats-1].
    task automatic mem_write_burst(input logic [31:0] addr, input logic [31:0] data[], input logic [3:0] strb = 4'hf);
        int beats = data.size();
        @(posedge clk);
        mem_axi.awvalid <= 1'b1;
        mem_axi.awaddr  <= addr;
        mem_axi.awlen   <= 8'(beats - 1);
        mem_axi.awid    <= 8'h5a;
        do @(posedge clk); while (!mem_axi.awready);
        mem_axi.awvalid <= 1'b0;
        for (int i = 0; i < beats; i++) begin
            mem_axi.wvalid <= 1'b1;
            mem_axi.wdata  <= data[i];
            mem_axi.wstrb  <= strb;
            mem_axi.wlast  <= (i == beats - 1);
            do @(posedge clk); while (!mem_axi.wready);
        end
        mem_axi.wvalid <= 1'b0;
        mem_axi.wlast  <= 1'b0;
        mem_axi.bready <= 1'b1;
        do @(posedge clk); while (!mem_axi.bvalid);
        if (mem_axi.bresp !== 2'b00 || mem_axi.bid !== 8'h5a) begin
            $display("FAIL: memory write at 0x%08x: bresp %0d bid 0x%02x", addr, mem_axi.bresp, mem_axi.bid);
            errors++;
        end
        mem_axi.bready <= 1'b0;
    endtask

    task automatic mem_write(input logic [31:0] addr, input logic [31:0] data, input logic [3:0] strb = 4'hf);
        logic [31:0] d[] = new[1];
        d[0] = data;
        mem_write_burst(addr, d, strb);
    endtask

    // Incrementing burst read. rready is dropped every third cycle.
    task automatic mem_read_burst(input logic [31:0] addr, input int beats, output logic [31:0] data[]);
        int got = 0;
        int cycle = 0;
        data = new[beats];
        @(posedge clk);
        mem_axi.arvalid <= 1'b1;
        mem_axi.araddr  <= addr;
        mem_axi.arlen   <= 8'(beats - 1);
        mem_axi.arid    <= 8'ha5;
        do @(posedge clk); while (!mem_axi.arready);
        mem_axi.arvalid <= 1'b0;
        mem_axi.rready  <= 1'b1;
        while (got < beats) begin
            @(posedge clk);
            if (mem_axi.rvalid && mem_axi.rready) begin
                data[got] = mem_axi.rdata;
                if (mem_axi.rresp !== 2'b00 || mem_axi.rid !== 8'ha5 || mem_axi.rlast !== (got == beats - 1)) begin
                    $display("FAIL: memory read at 0x%08x beat %0d: rresp %0d rid 0x%02x rlast %0d",
                             addr, got, mem_axi.rresp, mem_axi.rid, mem_axi.rlast);
                    errors++;
                end
                got++;
            end
            cycle++;
            mem_axi.rready <= (cycle % 3 != 2);
        end
        mem_axi.rready <= 1'b0;
    endtask

    task automatic mem_read(input logic [31:0] addr, output logic [31:0] data);
        logic [31:0] d[];
        mem_read_burst(addr, 1, d);
        data = d[0];
    endtask

    task automatic mem_expect(input string what, input logic [31:0] addr, input logic [31:0] expected);
        logic [31:0] data;
        mem_read(addr, data);
        check(what, data, expected);
    endtask

    // ---------------------------------------------------------------------
    // Flash helpers
    // ---------------------------------------------------------------------
    // Runs one flash operation and returns OP_STATUS. Clears OP_STATUS afterwards.
    task automatic flash_op(input logic [31:0] base, input int op, input logic [31:0] page, output logic [31:0] status);
        int polls = 0;
        reg_write(base + PAGE_NUM, page);
        reg_write(base + FL_CONTROL, (op << 1) | 1);
        do begin
            reg_read(base + OP_STATUS, status);
            polls++;
        end while (!status[0] && polls < 1000);
        if (!status[0]) begin
            $display("FAIL: flash operation %0d on page %0d did not finish", op, page);
            errors++;
        end
        reg_expect("START cleared", base + FL_CONTROL, op << 1);
        reg_expect("REGWEN set when idle", base + CTRL_REGWEN, 32'h1);
        reg_write(base + OP_STATUS, 32'h0);
        reg_expect("OP_STATUS cleared", base + OP_STATUS, 32'h0);
    endtask

    task automatic flash_op_ok(input logic [31:0] base, input int op, input logic [31:0] page);
        logic [31:0] status;
        flash_op(base, op, page, status);
        check($sformatf("flash op %0d page %0d status", op, page), status, 32'h1);
    endtask

    function automatic logic [31:0] pattern(input int seed, input int idx);
        return 32'h1234_5678 * (seed + 1) + 32'h0101_0101 * idx + {idx[7:0], 24'h0};
    endfunction

    task automatic flash_test(input string name, input logic [31:0] base, input logic [31:0] mem_base, input int seed);
        logic [31:0] data;
        logic [31:0] status;
        logic [31:0] burst[];
        logic [31:0] rd[];

        $display("[%0t] %s: registers", $time, name);
        reg_expect("FLASH_SIZE", base + FLASH_SIZE, FLASH_BYTES);
        reg_expect("PAGE_SIZE", base + PAGE_SIZE, 32'd256);
        reg_expect("CTRL_REGWEN", base + CTRL_REGWEN, 32'h1);
        reg_expect("OP_STATUS", base + OP_STATUS, 32'h0);
        reg_expect("FL_INTERRUPT_STATE", base + FL_INTERRUPT_STATE, 32'h0);
        reg_write(base + PAGE_ADDR, 32'hdead_beef);
        reg_expect("PAGE_ADDR", base + PAGE_ADDR, 32'hdead_beef);

        $display("[%0t] %s: erased flash reads as 0xFF", $time, name);
        for (int i = 0; i < 64; i++) reg_write(base + FLASH_BUF + 4 * i, 32'h0);
        flash_op_ok(base, OP_READ, 32'd5);
        for (int i = 0; i < 64; i++) reg_expect("erased page", base + FLASH_BUF + 4 * i, 32'hffff_ffff);
        mem_expect("erased memory", mem_base + 32'h500, 32'hffff_ffff);
        mem_expect("erased memory, last word", mem_base + FLASH_BYTES - 4, 32'hffff_ffff);
        reg_expect("EVENT set", base + FL_INTERRUPT_STATE, 32'h2);
        reg_write(base + FL_INTERRUPT_STATE, 32'h2);
        reg_expect("EVENT cleared", base + FL_INTERRUPT_STATE, 32'h0);

        $display("[%0t] %s: write page, read through memory port", $time, name);
        for (int i = 0; i < 64; i++) reg_write(base + FLASH_BUF + 4 * i, pattern(seed, i));
        for (int i = 0; i < 64; i++) reg_expect("FLASH_BUF", base + FLASH_BUF + 4 * i, pattern(seed, i));
        flash_op_ok(base, OP_WRITE, 32'd5);
        mem_read_burst(mem_base + 32'h500, 64, rd);
        for (int i = 0; i < 64; i++) check("page 5 through memory port", rd[i], pattern(seed, i));
        mem_expect("page 4 untouched", mem_base + 32'h4fc, 32'hffff_ffff);
        mem_expect("page 6 untouched", mem_base + 32'h600, 32'hffff_ffff);

        $display("[%0t] %s: byte writes to FLASH_BUF", $time, name);
        reg_write(base + FLASH_BUF + 4 * 3, 32'h0);
        reg_write_strb(base + FLASH_BUF + 4 * 3, 32'h0000_aa00, 4'b0010);
        reg_write_strb(base + FLASH_BUF + 4 * 3, 32'hbb00_0000, 4'b1000);
        reg_expect("byte writes", base + FLASH_BUF + 4 * 3, 32'hbb00_aa00);

        $display("[%0t] %s: read page back", $time, name);
        for (int i = 0; i < 64; i++) reg_write(base + FLASH_BUF + 4 * i, 32'h0);
        flash_op_ok(base, OP_READ, 32'd5);
        for (int i = 0; i < 64; i++) reg_expect("page 5 read", base + FLASH_BUF + 4 * i, pattern(seed, i));

        $display("[%0t] %s: write through memory port, read through controller", $time, name);
        // The last page of sector 3 and the first words of sector 4.
        burst = new[64];
        for (int i = 0; i < 64; i++) burst[i] = pattern(seed + 7, i);
        mem_write_burst(mem_base + 32'h3f00, burst);
        burst = new[16];
        for (int i = 0; i < 16; i++) burst[i] = pattern(seed + 7, 64 + i);
        mem_write_burst(mem_base + 32'h4000, burst);
        flash_op_ok(base, OP_READ, 32'd63);
        for (int i = 0; i < 64; i++) reg_expect("page 63 read", base + FLASH_BUF + 4 * i, pattern(seed + 7, i));
        flash_op_ok(base, OP_READ, 32'd64);
        for (int i = 0; i < 16; i++) reg_expect("page 64 read", base + FLASH_BUF + 4 * i, pattern(seed + 7, 64 + i));
        reg_expect("page 64 rest", base + FLASH_BUF + 4 * 16, 32'hffff_ffff);
        // Byte strobes on the memory port.
        mem_write(mem_base + 32'h3000, 32'h0011_2233);
        mem_write(mem_base + 32'h3000, 32'haabb_ccdd, 4'b0101);
        mem_expect("memory port byte strobes", mem_base + 32'h3000, 32'h00bb_22dd);
        mem_write(mem_base + 32'h3004, 32'h4455_6677);
        mem_expect("memory port odd word", mem_base + 32'h3004, 32'h4455_6677);
        mem_expect("memory port even word kept", mem_base + 32'h3000, 32'h00bb_22dd);

        $display("[%0t] %s: erase sector", $time, name);
        flash_op_ok(base, OP_ERASE, 32'd50);
        mem_expect("sector 3 first word erased", mem_base + 32'h3000, 32'hffff_ffff);
        mem_expect("sector 3 last word erased", mem_base + 32'h3ffc, 32'hffff_ffff);
        mem_expect("sector 4 kept", mem_base + 32'h4000, pattern(seed + 7, 64));
        mem_expect("sector 2 kept", mem_base + 32'h2ffc, 32'hffff_ffff);
        mem_expect("page 5 kept", mem_base + 32'h500, pattern(seed, 0));

        $display("[%0t] %s: last page", $time, name);
        for (int i = 0; i < 64; i++) reg_write(base + FLASH_BUF + 4 * i, pattern(seed + 3, i));
        flash_op_ok(base, OP_WRITE, FLASH_BYTES / 256 - 1);
        mem_expect("last word", mem_base + FLASH_BYTES - 4, pattern(seed + 3, 63));
        mem_expect("first word unchanged", mem_base, 32'hffff_ffff);

        $display("[%0t] %s: errors and interrupts", $time, name);
        reg_write(base + FL_INTERRUPT_STATE, 32'h3);
        reg_write(base + FL_INTERRUPT_ENABLE, 32'h1);
        flash_op(base, OP_READ, FLASH_BYTES / 256, status);
        check("read past the end", status, 32'h3);
        reg_expect("ERROR and EVENT set", base + FL_INTERRUPT_STATE, 32'h3);
        reg_write(base + FL_INTERRUPT_STATE, 32'h1);
        reg_expect("ERROR cleared", base + FL_INTERRUPT_STATE, 32'h2);
        flash_op(base, OP_WRITE, 32'hffff_ffff, status);
        check("write past the end", status, 32'h5);
        flash_op(base, OP_ERASE, FLASH_BYTES / 256, status);
        check("erase past the end", status, 32'h9);
        flash_op(base, 0, 32'd0, status);
        check("invalid operation", status, 32'h3);
        reg_write(base + PAGE_SIZE, 32'd128);
        flash_op(base, OP_READ, 32'd0, status);
        check("small page size", status, 32'h3);
        reg_write(base + PAGE_SIZE, 32'd256);
        reg_write(base + FL_INTERRUPT_STATE, 32'h3);
        reg_write(base + FL_INTERRUPT_ENABLE, 32'h0);
    endtask

    // ---------------------------------------------------------------------
    // SoC core helpers
    // ---------------------------------------------------------------------
    logic [31:0] fw_words [SOC_SRAM_BYTES / 4];

    task automatic soc_expect_str(input string expected);
        logic [31:0] data;
        for (int i = 0; i < expected.len(); i++) begin
            int polls = 0;
            do begin
                reg_read(SOC_FIFO_POP, data);
                polls++;
            end while (!data[8] && polls < 20000);
            if (!data[8]) begin
                $display("FAIL: no output from the SoC core, expected \"%s\" (at %0d)", expected, i);
                errors++;
                return;
            end
            if (data[7:0] !== expected[i]) begin
                $display("FAIL: SoC core output byte %0d: got 0x%02x, expected 0x%02x", i, data[7:0], expected[i]);
                errors++;
            end
        end
    endtask

    task automatic soc_core_test();
        logic [31:0] data;
        logic [31:0] burst[];
        logic [31:0] rd[];
        int fw_len;
        string fw_hex;

        $display("[%0t] soc core: held in reset", $time);
        reg_expect("soc_core_control", SOC_CORE_CONTROL, 32'h3);
        reg_expect("soc_core_status in reset", SOC_CORE_STATUS, 32'h0);
        reg_expect("soc_fifo_status", SOC_FIFO_STATUS, 32'h5);
        reg_expect("soc_fifo_pop empty", SOC_FIFO_POP, 32'h0);

        $display("[%0t] soc core: load firmware", $time);
        if (!$value$plusargs("SOC_CORE_FW=%s", fw_hex)) begin
            $display("FAIL: +SOC_CORE_FW=<hex file> is required");
            errors++;
            return;
        end
        for (int i = 0; i < $size(fw_words); i++) fw_words[i] = 'x;
        $readmemh(fw_hex, fw_words);
        fw_len = 0;
        while (fw_len < $size(fw_words) && !$isunknown(fw_words[fw_len])) fw_len++;
        $display("firmware: %0d words", fw_len);
        for (int offset = 0; offset < fw_len; offset += 16) begin
            int n = (fw_len - offset < 16) ? fw_len - offset : 16;
            burst = new[n];
            for (int i = 0; i < n; i++) burst[i] = fw_words[offset + i];
            mem_write_burst(SOC_SRAM_MEM + 4 * offset, burst);
        end
        mem_read_burst(SOC_SRAM_MEM, fw_len, rd);
        for (int i = 0; i < fw_len; i++) check("firmware read back", rd[i], fw_words[i]);
        mem_write(SOC_SRAM_MEM + SOC_SRAM_BYTES - 4, 32'hcafe_f00d);
        mem_expect("SRAM last word", SOC_SRAM_MEM + SOC_SRAM_BYTES - 4, 32'hcafe_f00d);
        mem_expect("past the SRAM reads zero", SOC_SRAM_MEM + SOC_SRAM_BYTES, 32'h0);
        mem_expect("unused slot reads zero", MEM_BASE + 32'h0300_0000, 32'h0);

        $display("[%0t] soc core: other FW_EXEC_CTRL bits do not release it", $time);
        fw_exec_ctrl = '1;
        fw_exec_ctrl[3] = 1'b0;
        reg_expect("fw_exec_ctrl[0]", FW_EXEC_CTRL, 32'hffff_fff7);
        reg_expect("fw_exec_ctrl[3]", FW_EXEC_CTRL + 12, 32'hffff_ffff);
        reg_expect("still in reset", SOC_CORE_STATUS, 32'h0);
        fw_exec_ctrl = '0;

        $display("[%0t] soc core: release with FW_EXEC_CTRL[3]", $time);
        fw_exec_ctrl[3] = 1'b1;
        reg_expect("running", SOC_CORE_STATUS, 32'h1);
        soc_expect_str("SOC CORE\n04fed79d 000d2f16 00000047\n");

        $display("[%0t] soc core: echo", $time);
        reg_write(SOC_FIFO_PUSH, "a");
        reg_write(SOC_FIFO_PUSH, "b");
        reg_write(SOC_FIFO_PUSH, "c");
        soc_expect_str("abc");
        reg_expect("soc_fifo_status after echo", SOC_FIFO_STATUS, 32'h5);

        $display("[%0t] soc core: trap", $time);
        reg_write(SOC_FIFO_PUSH, "q");
        repeat (400) @(posedge clk);
        reg_expect("trapped", SOC_CORE_STATUS, 32'h3);

        $display("[%0t] soc core: reset by clearing FW_EXEC_CTRL[3], select bit 100", $time);
        fw_exec_ctrl[3] = 1'b0;
        reg_expect("in reset again", SOC_CORE_STATUS, 32'h0);
        reg_write(SOC_CORE_CONTROL, 32'd100);
        fw_exec_ctrl[3] = 1'b1;
        reg_expect("bit 3 no longer selected", SOC_CORE_STATUS, 32'h0);
        fw_exec_ctrl[100] = 1'b1;
        reg_expect("running on bit 100", SOC_CORE_STATUS, 32'h1);
        soc_expect_str("SOC CORE\n04fed79d 000d2f16 00000047\n");

        $display("[%0t] soc core: force_reset and force_run", $time);
        reg_write(SOC_CORE_CONTROL, 32'd100 | 32'h100);
        reg_expect("force_reset", SOC_CORE_STATUS, 32'h0);
        fw_exec_ctrl = '0;
        reg_write(SOC_CORE_CONTROL, 32'd100 | 32'h200);
        reg_expect("force_run", SOC_CORE_STATUS, 32'h1);
        soc_expect_str("SOC CORE\n");
        reg_write(SOC_CORE_CONTROL, 32'd100 | 32'h300);
        reg_expect("force_reset wins", SOC_CORE_STATUS, 32'h0);
        reg_write(SOC_CORE_CONTROL, 32'd3);

        $display("[%0t] soc core: bytes pushed while in reset are dropped", $time);
        // Drain what the last run left behind.
        do reg_read(SOC_FIFO_POP, data); while (data[8]);
        reg_write(SOC_FIFO_PUSH, "x");
        reg_expect("to-core FIFO stays empty in reset", SOC_FIFO_STATUS, 32'h5);

        $display("[%0t] soc core: output FIFO fills without a reader", $time);
        fw_exec_ctrl[3] = 1'b1;
        soc_expect_str("SOC CORE\n04fed79d 000d2f16 00000047\n");
        // The echo of 64 bytes stays in the 1024 entry FIFO until read.
        for (int i = 0; i < 40; i++) reg_write(SOC_FIFO_PUSH, 8'h30 + i);
        repeat (4000) @(posedge clk);
        for (int i = 0; i < 40; i++) begin
            reg_read(SOC_FIFO_POP, data);
            check("queued echo", data, 32'h130 + i);
        end
        fw_exec_ctrl = '0;
    endtask

    // ---------------------------------------------------------------------
    // Test sequence
    // ---------------------------------------------------------------------
    initial begin
        logic [31:0] data;

        repeat (10) @(posedge clk);
        rst <= 1'b0;
        repeat (10) @(posedge clk);

        $display("[%0t] existing registers", $time);
        reg_expect("fpga_magic", FPGA_MAGIC, 32'h5254_5043);
        reg_expect("fpga_features", FPGA_FEATURES, 32'h7);
        reg_expect("software flash PAGE_SIZE", OLD_PRIMARY_FLASH + PAGE_SIZE, 32'd256);
        reg_expect("software flash FLASH_SIZE", OLD_PRIMARY_FLASH + FLASH_SIZE, 32'h0100_0000);
        reg_write(OLD_PRIMARY_FLASH + FL_CONTROL, 32'h3);
        reg_expect("software flash START is not cleared by hardware", OLD_PRIMARY_FLASH + FL_CONTROL, 32'h3);
        reg_write(CDMA_AXI_USER, 32'h1234_5678);
        reg_expect("cdma_axi_user", CDMA_AXI_USER, 32'h1234_5678);
        check("cdma_axi_user output", hwif_out.interface_regs.cdma_axi_user.cdma_axi_user.value, 32'h1234_5678);

        flash_test("primary flash", PRIMARY_FLASH, PRIMARY_FLASH_MEM, 0);
        check("primary flash irq low", {31'b0, primary_flash_irq}, 32'h0);

        $display("[%0t] flash interrupt output", $time);
        reg_write(PRIMARY_FLASH + FL_INTERRUPT_ENABLE, 32'h2);
        flash_op_ok(PRIMARY_FLASH, OP_READ, 32'd0);
        check("primary flash irq high", {31'b0, primary_flash_irq}, 32'h1);
        check("secondary flash irq low", {31'b0, secondary_flash_irq}, 32'h0);
        reg_write(PRIMARY_FLASH + FL_INTERRUPT_STATE, 32'h2);
        repeat (2) @(posedge clk);
        check("primary flash irq cleared", {31'b0, primary_flash_irq}, 32'h0);
        reg_write(PRIMARY_FLASH + FL_INTERRUPT_ENABLE, 32'h0);

        flash_test("secondary flash", SECONDARY_FLASH, SECONDARY_FLASH_MEM, 11);

        $display("[%0t] flashes are independent", $time);
        mem_expect("primary page 5", PRIMARY_FLASH_MEM + 32'h500, pattern(0, 0));
        mem_expect("secondary page 5", SECONDARY_FLASH_MEM + 32'h500, pattern(11, 0));

        $display("[%0t] writes are ignored while an operation runs", $time);
        reg_write(PRIMARY_FLASH + PAGE_NUM, 32'd5);
        reg_write(PRIMARY_FLASH + FL_CONTROL, (OP_ERASE << 1) | 1);
        reg_expect("REGWEN low while busy", PRIMARY_FLASH + CTRL_REGWEN, 32'h0);
        reg_write(PRIMARY_FLASH + PAGE_NUM, 32'd999);
        reg_write(PRIMARY_FLASH + FL_CONTROL, 32'h0);
        reg_expect("PAGE_NUM kept", PRIMARY_FLASH + PAGE_NUM, 32'd5);
        do reg_read(PRIMARY_FLASH + OP_STATUS, data); while (!data[0]);
        check("erase status", data, 32'h1);
        reg_write(PRIMARY_FLASH + OP_STATUS, 32'h0);
        mem_expect("page 5 erased", PRIMARY_FLASH_MEM + 32'h500, 32'hffff_ffff);

        soc_core_test();

        $display("[%0t] flash contents survive reset", $time);
        @(posedge clk);
        rst <= 1'b1;
        repeat (5) @(posedge clk);
        rst <= 1'b0;
        repeat (5) @(posedge clk);
        reg_expect("OP_STATUS after reset", SECONDARY_FLASH + OP_STATUS, 32'h0);
        mem_expect("secondary page 5 after reset", SECONDARY_FLASH_MEM + 32'h500, pattern(11, 0));
        mem_expect("SoC SRAM after reset", SOC_SRAM_MEM, fw_words[0]);

        if (errors == 0) begin
            $display("TEST PASSED");
        end else begin
            $display("TEST FAILED: %0d errors", errors);
        end
        $finish;
    end

    initial begin
        #200ms;
        $display("TEST FAILED: timeout");
        $finish;
    end
endmodule
