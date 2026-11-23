// Licensed under the Apache-2.0 license

module dual_itrng_regs_tb;
    import caliptra_fpga_realtime_regs_pkg::*;
    logic clk = 0;
    logic rst = 1;
    axi4lite_intf #(.DATA_WIDTH(32)) bus();
    caliptra_fpga_realtime_regs__in_t hwif_in;
    caliptra_fpga_realtime_regs__out_t hwif_out;

    always #5 clk = ~clk;
    always_comb begin
        hwif_in = '{default: '0};
`ifdef CALIPTRA_FPGA_DUAL_ITRNG
        hwif_in.interface_regs.status.dual_itrng_capable.next = 1;
        hwif_in.interface_regs.entropy_ctrl.itrng1_en.swwe =
            !hwif_out.interface_regs.control.cptra_ss_rst_b.value &&
            !hwif_out.interface_regs.control.cptra_pwrgood.value;
`endif
    end
    caliptra_fpga_realtime_regs dut (.clk, .rst, .s_axil(bus), .hwif_in, .hwif_out);

    task automatic write_reg(input logic [31:0] addr, data);
        @(negedge clk);
        bus.AWADDR = 'ha4010000 + addr;
        bus.WDATA = data;
        bus.AWVALID = 1;
        bus.WVALID = 1;
        fork
            begin
                do @(posedge clk); while (!bus.AWREADY);
                @(negedge clk);
                bus.AWVALID = 0;
            end
            begin
                do @(posedge clk); while (!bus.WREADY);
                @(negedge clk);
                bus.WVALID = 0;
            end
        join
        do @(posedge clk); while (!bus.BVALID);
        if (bus.BRESP != 0) $fatal(1, "AXI write failed");
        @(negedge clk);
    endtask

    initial begin
        bus.AWVALID = 0;
        bus.WVALID = 0;
        bus.ARVALID = 0;
        bus.AWADDR = 0;
        bus.WDATA = 0;
        bus.WSTRB = '1;
        bus.AWPROT = 0;
        bus.ARPROT = 0;
        bus.ARADDR = 0;
        bus.RREADY = 1;
        bus.BREADY = 1;
        repeat (4) @(negedge clk);
        rst = 0;
        if (hwif_out.interface_regs.entropy_ctrl.itrng1_en.value != 0)
            $fatal(1, "ES1 strap must default to zero");
        write_reg('h8, 0);
        write_reg('h1c, 1);
`ifdef CALIPTRA_FPGA_DUAL_ITRNG
        if (hwif_out.interface_regs.entropy_ctrl.itrng1_en.value != 1)
            $fatal(1, "Cold reset must permit ES1 selection");
        write_reg('h8, 1);
        write_reg('h1c, 0);
        if (hwif_out.interface_regs.entropy_ctrl.itrng1_en.value != 1)
            $fatal(1, "Power-good alone must block strap writes");
        write_reg('h8, 2);
        write_reg('h1c, 0);
        if (hwif_out.interface_regs.entropy_ctrl.itrng1_en.value != 1)
            $fatal(1, "Subsystem reset release must block strap writes");
        write_reg('h8, 0);
        write_reg('h1c, 0);
        if (hwif_out.interface_regs.entropy_ctrl.itrng1_en.value != 0)
            $fatal(1, "Cold reset must permit clearing ES1");
`else
        if (hwif_out.interface_regs.entropy_ctrl.itrng1_en.value != 0)
            $fatal(1, "Standard image must ignore ES1 strap writes");
`endif
        $display("PASS: ES1 cold-reset strap behavior");
        $finish;
    end

    initial begin
        #10000;
        $fatal(1, "AXI test deadline exceeded");
    end
endmodule
