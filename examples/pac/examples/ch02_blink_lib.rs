//! 第 2 章例程 2：点灯的工程化形态——寄存器样板收进 crate 库（lib.rs）。
//!
//! 与 ch02_blink.rs 硬件行为完全相同，但门控/退模拟/方向三步进了
//! `board::Led::new`，延时进了 `board::delay_ms`（SysTick 忙等，
//! 复位默认 HCLK = 4 MHz 下毫秒级标定）。裸寄存器教学版请看
//! ch02_blink.rs——两个例程对照着读，就是"从例程到库"的演进本身。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch02_blink_lib -p cw32-book-pac
//! 状态：仅编译未上板；寄存器序列与 ch02_blink.rs 逐字同源，后者与
//! cw32l012-hal 实测例 blink_pc13 同源（TESTING.md：PC13 500ms 方波）。

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use cw32_book_pac::board::{Led, delay_ms};
use cw32l012_pac::Peripherals;

#[entry]
fn main() -> ! {
    // 外设单例仍由例程取得——"谁拥有外设"这件事不藏进库里
    let p = Peripherals::take().unwrap();

    // 构造即完成 ch02 的三步：门控（KEY 写保护）→ 退模拟态 → 方向输出
    let mut led = Led::new(&p);

    loop {
        led.toggle(); // TOG 位翻转（与 ch02_blink 同一寄存器写）
        delay_ms(500); // SysTick 忙等 500ms（逐毫秒单次倒数）
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
