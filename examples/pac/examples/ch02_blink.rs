//! 第 2 章例程：PAC 寄存器直写点灯（占位脚手架——本章写作时替换为
//! 完整的"裸寄存器点灯"版本，含 GPIOC 门控 + KEY 写保护逐行讲解）。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch02_blink
//! 状态：仅编译通过，未上板。

#![no_std]
#![no_main]

use cortex_m_rt::entry;

#[entry]
fn main() -> ! {
    // 真正"用到"PAC：未被引用的依赖 rustc 不会交给链接器，
    // link.x 的 EXTERN(__INTERRUPTS) 就无对象可拉（向量表缺失）。
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
