// Licensed under the Apache-2.0 license

// Flash controller backed by on-chip RAM.
//
// Registers (hw_flash_ctrl_regs in caliptra_fpga_realtime_regs.rdl):
//   0x000 FL_INTERRUPT_STATE   [0] ERROR, [1] EVENT. Write 1 to clear.
//   0x004 FL_INTERRUPT_ENABLE  [0] ERROR, [1] EVENT.
//   0x008 PAGE_SIZE            Operations fail if this is less than 256.
//   0x00C PAGE_NUM
//   0x010 PAGE_ADDR            Storage only.
//   0x014 FL_CONTROL           [0] START, [2:1] OP (1 read, 2 write, 3 erase).
//   0x018 OP_STATUS            [0] DONE, [3:1] ERR.
//   0x01C CTRL_REGWEN          [0] EN. Low while an operation is running.
//   0x020 FLASH_SIZE
//   0x100 FLASH_BUF[64]        Page buffer.
//
// A read copies a 256 byte page into FLASH_BUF, a write copies FLASH_BUF into
// the page, and an erase sets the 4 KiB sector containing the page to 0xFF.
//
// The RAM powers up as zero and holds the complement of the flash contents,
// so the flash reads as erased (0xFF) after the FPGA is configured. The
// contents are kept across rst.
module fpga_hw_flash #(
    // Flash size in bytes. Must be a power of two and at least 4096.
    parameter int SIZE_BYTES = 4 * 1024 * 1024,
    localparam int MEM_AW = $clog2(SIZE_BYTES)
) (
    input  logic        clk,
    // Synchronous, active high. Resets the registers and aborts an operation.
    input  logic        rst,

    // Register bus (PeakRDL external block interface).
    input  logic        req,
    input  logic        req_is_wr,
    input  logic [8:0]  addr,
    input  logic [31:0] wr_data,
    input  logic [31:0] wr_biten,
    output logic        rd_ack,
    output logic [31:0] rd_data,
    output logic        wr_ack,

    // Direct access to the flash contents.
    // mem_addr is a byte address; bits [1:0] are ignored.
    // mem_rdata is valid in the cycle after mem_en.
    input  logic              mem_en,
    input  logic [3:0]        mem_we,
    input  logic [MEM_AW-1:0] mem_addr,
    input  logic [31:0]       mem_wdata,
    output logic [31:0]       mem_rdata,

    output logic        irq
);
    localparam int PAGE_BYTES    = 256;
    localparam int SECTOR_BYTES  = 4096;
    // The RAM is 64 bits wide so that it fills the 72 bit UltraRAM words.
    localparam int RAM_AW        = MEM_AW - 3;
    localparam int WORDS_PER_PAGE   = PAGE_BYTES / 8;
    localparam int WORDS_PER_SECTOR = SECTOR_BYTES / 8;
    localparam int PAGE_CNT_W    = $clog2(WORDS_PER_PAGE);
    localparam int SECTOR_CNT_W  = $clog2(WORDS_PER_SECTOR);
    localparam int NUM_PAGES     = SIZE_BYTES / PAGE_BYTES;
    localparam int PAGE_NUM_W    = $clog2(NUM_PAGES);

    localparam logic [1:0] OP_READ  = 2'd1;
    localparam logic [1:0] OP_WRITE = 2'd2;
    localparam logic [1:0] OP_ERASE = 2'd3;

    localparam logic [2:0] ERR_NONE  = 3'd0;
    localparam logic [2:0] ERR_READ  = 3'd1;
    localparam logic [2:0] ERR_WRITE = 3'd2;
    localparam logic [2:0] ERR_ERASE = 3'd4;

    typedef enum logic [2:0] {
        S_IDLE,
        S_CHECK,
        S_READ_ADDR,
        S_READ_DATA,
        S_WRITE,
        S_ERASE,
        S_DONE
    } state_e;

    state_e state;

    // Registers
    logic [1:0]  intr_state;
    logic [1:0]  intr_enable;
    logic [31:0] page_size;
    logic [31:0] page_num;
    logic [31:0] page_addr;
    logic        start;
    logic [1:0]  op;
    logic        done;
    logic [2:0]  err;

    // An operation runs from the write that sets START until START is cleared.
    logic busy;
    assign busy = start || (state != S_IDLE);

    logic [2:0]              err_code;
    logic [SECTOR_CNT_W-1:0] cnt;

    assign irq = |(intr_state & intr_enable);

    // Page buffer. Even and odd words are kept apart so that a 64 bit RAM
    // word is read or written in one cycle.
    logic [31:0] buf_lo [WORDS_PER_PAGE];
    logic [31:0] buf_hi [WORDS_PER_PAGE];
    logic [PAGE_CNT_W-1:0] buf_idx;
    logic [31:0] buf_lo_rdata;
    logic [31:0] buf_hi_rdata;
    logic        buf_lo_we;
    logic        buf_hi_we;
    logic [31:0] buf_lo_wdata;
    logic [31:0] buf_hi_wdata;

    // RAM port used by the controller
    logic              ram_en;
    logic              ram_we;
    logic [RAM_AW-1:0] ram_addr;
    logic [63:0]       ram_wdata;
    logic [63:0]       ram_rdata;

    // Register bus decode
    logic       sw_wr;
    logic       sw_buf_sel;
    logic [5:0] sw_word;
    assign sw_wr      = req && req_is_wr;
    assign sw_buf_sel = addr[8];
    assign sw_word    = addr[7:2];

    // Current value of the addressed register and the value after a write.
    logic [31:0] reg_rdata;
    logic [31:0] reg_wdata;
    assign reg_wdata = (reg_rdata & ~wr_biten) | (wr_data & wr_biten);

    assign buf_idx      = busy ? cnt[PAGE_CNT_W-1:0] : sw_word[5:1];
    assign buf_lo_rdata = buf_lo[buf_idx];
    assign buf_hi_rdata = buf_hi[buf_idx];

    always_comb begin
        buf_lo_we    = 1'b0;
        buf_hi_we    = 1'b0;
        buf_lo_wdata = ~ram_rdata[31:0];
        buf_hi_wdata = ~ram_rdata[63:32];
        if (state == S_READ_DATA) begin
            buf_lo_we = 1'b1;
            buf_hi_we = 1'b1;
        end else if (!busy && sw_wr && sw_buf_sel) begin
            buf_lo_we    = !sw_word[0];
            buf_hi_we    = sw_word[0];
            buf_lo_wdata = reg_wdata;
            buf_hi_wdata = reg_wdata;
        end
    end

    always_ff @(posedge clk) begin
        if (buf_lo_we) buf_lo[buf_idx] <= buf_lo_wdata;
        if (buf_hi_we) buf_hi[buf_idx] <= buf_hi_wdata;
    end

    // Register read
    always_comb begin
        reg_rdata = '0;
        if (sw_buf_sel) begin
            reg_rdata = sw_word[0] ? buf_hi_rdata : buf_lo_rdata;
        end else begin
            unique case (addr[7:2])
                6'h00:   reg_rdata = {30'b0, intr_state};
                6'h01:   reg_rdata = {30'b0, intr_enable};
                6'h02:   reg_rdata = page_size;
                6'h03:   reg_rdata = page_num;
                6'h04:   reg_rdata = page_addr;
                6'h05:   reg_rdata = {29'b0, op, start};
                6'h06:   reg_rdata = {28'b0, err, done};
                6'h07:   reg_rdata = {31'b0, !busy};
                6'h08:   reg_rdata = SIZE_BYTES;
                default: reg_rdata = '0;
            endcase
        end
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            rd_ack  <= 1'b0;
            wr_ack  <= 1'b0;
            rd_data <= '0;
        end else begin
            rd_ack  <= req && !req_is_wr;
            wr_ack  <= req && req_is_wr;
            // While an operation runs the buffer read port belongs to the controller.
            rd_data <= (sw_buf_sel && busy) ? '0 : reg_rdata;
        end
    end

    // Operation parameter checks
    logic page_valid;
    assign page_valid = (page_num < NUM_PAGES) && (page_size >= PAGE_BYTES);

    logic [PAGE_NUM_W-1:0] page_idx;
    assign page_idx = page_num[PAGE_NUM_W-1:0];

    always_comb begin
        ram_en    = 1'b0;
        ram_we    = 1'b0;
        ram_addr  = {page_idx, cnt[PAGE_CNT_W-1:0]};
        ram_wdata = ~{buf_hi_rdata, buf_lo_rdata};
        unique case (state)
            S_READ_ADDR: begin
                ram_en = 1'b1;
            end
            S_WRITE: begin
                ram_en = 1'b1;
                ram_we = 1'b1;
            end
            S_ERASE: begin
                ram_en    = 1'b1;
                ram_we    = 1'b1;
                ram_addr  = {page_idx[PAGE_NUM_W-1:SECTOR_CNT_W-PAGE_CNT_W], cnt};
                ram_wdata = '0;
            end
            default: ;
        endcase
    end

    // Registers and controller state machine
    always_ff @(posedge clk) begin
        if (rst) begin
            state       <= S_IDLE;
            intr_state  <= '0;
            intr_enable <= '0;
            page_size   <= PAGE_BYTES;
            page_num    <= '0;
            page_addr   <= '0;
            start       <= 1'b0;
            op          <= '0;
            done        <= 1'b0;
            err         <= '0;
            err_code    <= ERR_NONE;
            cnt         <= '0;
        end else begin
            // Software writes
            if (sw_wr && !sw_buf_sel) begin
                unique case (addr[7:2])
                    6'h00: intr_state <= intr_state & ~(wr_data[1:0] & wr_biten[1:0]);
                    6'h01: intr_enable <= reg_wdata[1:0];
                    6'h02: if (!busy) page_size <= reg_wdata;
                    6'h03: if (!busy) page_num <= reg_wdata;
                    6'h04: page_addr <= reg_wdata;
                    6'h05: if (!busy) {op, start} <= reg_wdata[2:0];
                    6'h06: {err, done} <= reg_wdata[3:0];
                    default: ;
                endcase
            end

            // Controller. Its updates take priority over software writes.
            unique case (state)
                S_IDLE: begin
                    if (start) begin
                        state <= S_CHECK;
                    end
                end
                S_CHECK: begin
                    cnt <= '0;
                    unique case (op)
                        OP_READ: begin
                            err_code <= page_valid ? ERR_NONE : ERR_READ;
                            state    <= page_valid ? S_READ_ADDR : S_DONE;
                        end
                        OP_WRITE: begin
                            err_code <= page_valid ? ERR_NONE : ERR_WRITE;
                            state    <= page_valid ? S_WRITE : S_DONE;
                        end
                        OP_ERASE: begin
                            err_code <= page_valid ? ERR_NONE : ERR_ERASE;
                            state    <= page_valid ? S_ERASE : S_DONE;
                        end
                        default: begin
                            err_code <= ERR_READ;
                            state    <= S_DONE;
                        end
                    endcase
                end
                S_READ_ADDR: begin
                    state <= S_READ_DATA;
                end
                S_READ_DATA: begin
                    cnt <= cnt + 1'b1;
                    if (cnt[PAGE_CNT_W-1:0] == PAGE_CNT_W'(WORDS_PER_PAGE - 1)) begin
                        state <= S_DONE;
                    end else begin
                        state <= S_READ_ADDR;
                    end
                end
                S_WRITE: begin
                    cnt <= cnt + 1'b1;
                    if (cnt[PAGE_CNT_W-1:0] == PAGE_CNT_W'(WORDS_PER_PAGE - 1)) begin
                        state <= S_DONE;
                    end
                end
                S_ERASE: begin
                    cnt <= cnt + 1'b1;
                    if (cnt == SECTOR_CNT_W'(WORDS_PER_SECTOR - 1)) begin
                        state <= S_DONE;
                    end
                end
                S_DONE: begin
                    start         <= 1'b0;
                    done          <= 1'b1;
                    err           <= err_code;
                    intr_state[1] <= 1'b1;
                    if (err_code != ERR_NONE) begin
                        intr_state[0] <= 1'b1;
                    end
                    state <= S_IDLE;
                end
                default: begin
                    state <= S_IDLE;
                end
            endcase
        end
    end

    // Direct access port. Each 32 bit access maps to half of a RAM word.
    logic              mem_half_f;
    logic [63:0]       mem_ram_rdata;
    logic [7:0]        mem_ram_we;
    assign mem_ram_we = mem_addr[2] ? {mem_we, 4'b0} : {4'b0, mem_we};
    assign mem_rdata  = mem_half_f ? ~mem_ram_rdata[63:32] : ~mem_ram_rdata[31:0];

    always_ff @(posedge clk) begin
        if (mem_en) begin
            mem_half_f <= mem_addr[2];
        end
    end

    // Port A is the direct access port, port B is the controller.
    xpm_memory_tdpram #(
        .ADDR_WIDTH_A(RAM_AW),
        .ADDR_WIDTH_B(RAM_AW),
        .AUTO_SLEEP_TIME(0),
        .BYTE_WRITE_WIDTH_A(8),
        .BYTE_WRITE_WIDTH_B(8),
        .CASCADE_HEIGHT(0),
        .CLOCKING_MODE("common_clock"),
        .ECC_MODE("no_ecc"),
        .MEMORY_INIT_FILE("none"),
        .MEMORY_INIT_PARAM("0"),
        .MEMORY_OPTIMIZATION("false"),
        .MEMORY_PRIMITIVE("ultra"),
        .MEMORY_SIZE(SIZE_BYTES * 8),
        .MESSAGE_CONTROL(0),
        .READ_DATA_WIDTH_A(64),
        .READ_DATA_WIDTH_B(64),
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
        .WRITE_DATA_WIDTH_A(64),
        .WRITE_DATA_WIDTH_B(64),
        .WRITE_MODE_A("no_change"),
        .WRITE_MODE_B("no_change"),
        .WRITE_PROTECT(1)
    ) flash_ram (
        .dbiterra(),
        .dbiterrb(),
        .douta(mem_ram_rdata),
        .doutb(ram_rdata),
        .sbiterra(),
        .sbiterrb(),
        .addra(mem_addr[MEM_AW-1:3]),
        .addrb(ram_addr),
        .clka(clk),
        .clkb(clk),
        .dina(~{mem_wdata, mem_wdata}),
        .dinb(ram_wdata),
        .ena(mem_en),
        .enb(ram_en),
        .injectdbiterra(1'b0),
        .injectdbiterrb(1'b0),
        .injectsbiterra(1'b0),
        .injectsbiterrb(1'b0),
        .regcea(1'b1),
        .regceb(1'b1),
        .rsta(1'b0),
        .rstb(1'b0),
        .sleep(1'b0),
        .wea(mem_ram_we),
        .web({8{ram_we}})
    );
endmodule
