//! 第 8 章例程 1：异步等待按键边沿，翻转 PC13 上的 LED。
//!
//! 按键（或杜邦线）接在 PA0 与 GND 之间：PA0 配上拉输入，按下 =
//! 下降沿。`Input::wait_for_falling_edge()` 把任务挂起，GPIOA 端口
//! 中断（IRQ5）到来才唤醒——等待期间 CPU 睡在执行器里。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch08_gpio_wait -p cw32-book-embassy
//! 状态：仅编译未上板（首次上板核对点见第 8 章正文）。

#![no_std]
#![no_main]

use embassy_executor::main;

use cw32_book_embassy as _; // 继承 lib.rs 的 panic handler（panic-probe 同款机制）
use embassy_cw32::{
    Config, Peripherals, SYSCTRL_KEY,
    gpio::{Input, Level, Output, Pin, Pull},
};

// 只有"被等待"的端口才需要绑中断：GPIOA 上有 Input 的等待。
// LED 是 Output，不产生中断，GPIOC 无需绑定。
embassy_cw32::bind_interrupts!(struct Irqs {
    GPIOA => embassy_cw32::gpio::GpioAIrq;
});

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk = 4MHz

    let p = Peripherals::take().unwrap();
    // AHBEN 是键控整字写（高 16 位必须 0x5A5A，否则写入无效）：
    // GPIOA(bit4) 与 GPIOC(bit6) 要写进同一次写——分两次各写一位，
    // 后一次会把前一次刚写的位清掉
    p.sysctrl.ahben.write(SYSCTRL_KEY | (1 << 4) | (1 << 6));

    let mut btn = Input::new(Pin::new(p.gpioa, 0), Pull::Up);
    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    loop {
        btn.wait_for_falling_edge().await; // 挂起，等 GPIOA 端口中断
        led.toggle();
    }
}
