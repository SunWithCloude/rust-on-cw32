//! 第 3 章例程：RTT 日志（占位脚手架——本章写作时替换为 defmt-rtt
//! + panic-probe 的完整版本）。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch03_rtt
//! 状态：仅编译通过，未上板。

#![no_std]
#![no_main]

use cortex_m_rt::entry;

#[entry]
fn main() -> ! {
    let _irq = cw32l012_pac::Interrupt::Uart1;

    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
