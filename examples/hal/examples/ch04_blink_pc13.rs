//! 第 4 章例程：板载 PC13 LED 闪烁（cw32l012-hal 阻塞式）。
//!
//! HAL GPIO 全路径：`Pin::new`（内部 Rcc::enable_gpio 走 SYSCTRL_AHBEN
//! KEY=0x5A5A 保护写）→ `into_output`（清 OPENDRAIN/PUR/PDR/ANALOG +
//! AF 归零 → DIR=0）→ `Output::toggle`（TOG 原子翻转）。
//! PC13 复位默认 ANALOG=1（模拟态），本例可验证 HAL 已正确清零。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch04_blink_pc13 -p cw32-book-hal
//! 状态：同源已验证（与 cw32l012-hal examples/blink_pc13.rs 逐行同源；
//!       TESTING.md 实测通过表：PC13 500ms 方波）。
//! 差异：本例按 edition 2024 写作 `#[unsafe(no_mangle)]`。

#![no_std]
#![no_main]

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

#[unsafe(no_mangle)]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };

    // 复位默认时钟：SysClk = HSI = 4MHz，HCLK = PCLK = 4MHz
    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks();

    // 板载 LED：PC13（低电平/高电平点亮视板卡而定，toggle 通吃两种接法）
    let mut led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);

    let mut delay = Delay::new(&clocks);

    loop {
        led.toggle();
        delay.delay_ms(500);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
