// Licensed under the Apache-2.0 license

//! Test firmware for the FPGA SoC core (fpga_soc_core.sv).
//!
//! Prints a banner and the result of a multiply and a divide, then echoes
//! every byte it receives. A received 'q' makes it execute ebreak, which
//! raises the trap status bit.

#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};

const DATA_OUT: *mut u32 = 0x1000_0000 as *mut u32;
const DATA_IN: *const u32 = 0x1000_0004 as *const u32;
const STATUS: *const u32 = 0x1000_0008 as *const u32;

const STATUS_OUT_FULL: u32 = 0x1;
const DATA_IN_VALID: u32 = 0x100;

// Read with read_volatile so that the arithmetic runs on the core.
static MUL_A: u32 = 12345;
static MUL_B: u32 = 6789;
static DIV_B: u32 = 97;

global_asm!(
    ".section .text.init",
    ".globl _start",
    "_start:",
    "    la sp, _stack_top",
    "    call main",
    "1:",
    "    j 1b",
);

fn put_char(c: u8) {
    // SAFETY: STATUS and DATA_OUT are registers of the SoC core.
    unsafe {
        while read_volatile(STATUS) & STATUS_OUT_FULL != 0 {}
        write_volatile(DATA_OUT, c as u32);
    }
}

fn put_str(s: &[u8]) {
    for c in s {
        put_char(*c);
    }
}

fn put_hex(v: u32) {
    for shift in (0..32).step_by(4).rev() {
        let digit = ((v >> shift) & 0xf) as u8;
        put_char(if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        });
    }
}

#[no_mangle]
pub extern "C" fn main() -> ! {
    put_str(b"SOC CORE\n");
    // SAFETY: the statics are valid for reads.
    let (a, b, d) = unsafe {
        (
            read_volatile(&MUL_A),
            read_volatile(&MUL_B),
            read_volatile(&DIV_B),
        )
    };
    let product = a.wrapping_mul(b);
    put_hex(product);
    put_char(b' ');
    put_hex(product / d);
    put_char(b' ');
    put_hex(product % d);
    put_char(b'\n');

    loop {
        // SAFETY: DATA_IN is a register of the SoC core.
        let input = unsafe { read_volatile(DATA_IN) };
        if input & DATA_IN_VALID != 0 {
            let c = input as u8;
            if c == b'q' {
                // SAFETY: ebreak stops the core, which is what the test wants.
                unsafe { asm!("ebreak") };
            }
            put_char(c);
        }
    }
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}
