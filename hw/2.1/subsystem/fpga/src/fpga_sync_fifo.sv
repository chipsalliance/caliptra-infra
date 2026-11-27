// Licensed under the Apache-2.0 license

// Synchronous first-word-fall-through FIFO.
// dout holds the oldest entry whenever empty is low.
// Writes while full and reads while empty are ignored.
module fpga_sync_fifo #(
    parameter int WIDTH = 8,
    // Must be a power of two.
    parameter int DEPTH = 256
) (
    input  logic             clk,
    // Synchronous, active high. Empties the FIFO.
    input  logic             rst,

    input  logic             wr_en,
    input  logic [WIDTH-1:0] din,
    output logic             full,

    input  logic             rd_en,
    output logic [WIDTH-1:0] dout,
    output logic             empty
);
    localparam int AW = $clog2(DEPTH);

    logic [WIDTH-1:0] mem [DEPTH];
    logic [AW:0] wptr;
    logic [AW:0] rptr;

    assign empty = (wptr == rptr);
    assign full  = (wptr[AW] != rptr[AW]) && (wptr[AW-1:0] == rptr[AW-1:0]);
    assign dout  = mem[rptr[AW-1:0]];

    always_ff @(posedge clk) begin
        if (wr_en && !full) begin
            mem[wptr[AW-1:0]] <= din;
        end
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            wptr <= '0;
            rptr <= '0;
        end else begin
            if (wr_en && !full) begin
                wptr <= wptr + 1'b1;
            end
            if (rd_en && !empty) begin
                rptr <= rptr + 1'b1;
            end
        end
    end
endmodule
