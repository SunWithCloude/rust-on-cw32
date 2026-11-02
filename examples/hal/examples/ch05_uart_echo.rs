//! 第 5 章例程：UART1 回显（PA02=TXD、PA03=RXD，AF1，115200-8-N-1）。
//!
//! 与 cw32l012-hal examples/uart_echo.rs 逐行同源（仅按 edition 2024
//! 改为 #[unsafe(no_mangle)] 并调整注释）。接线：MCU PA02 -> USB 串口
//! 适配器 RXD，MCU PA03 <- USB 串口适配器 TXD，共地。复位后发送欢迎
//! 消息，随后把收到的每个字节原样回显。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch05_uart_echo -p cw32-book-hal
//! 状态：仅编译未上板（生态仓 uart_echo 本身未列入 TESTING.md 实测通过表；
//!       TX 写路径已有 ecosystem 实测旁证：PA02 见波形、写入 13 字节）。

#![no_std]
#![no_main]

use cw32_book_hal as _; // 继承 lib.rs 的 panic handler（panic-probe 同款机制）
use cw32l012_hal::{
    gpio::{Af, Pin, PortId},
    rcc::Rcc,
    uart::Uart,
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

    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks(); // PCLK = 4MHz

    // UART1 引脚：PA02 = UART1_TXD，PA03 = UART1_RXD（AF1）
    let tx = Pin::new(p.gpioa, PortId::A, 2, &mut rcc).into_af_output(Af::Af1);
    let rx = Pin::new(p.gpioa, PortId::A, 3, &mut rcc).into_af_input(Af::Af1);

    let mut uart = Uart::new(p.uart1, tx, rx, &mut rcc, clocks.pclk_hz, 115_200);

    uart.write_str("CW32L012 Rust HAL ready\r\n");

    loop {
        if let Some(b) = uart.try_read_byte() {
            uart.write_byte(b);
        }
    }
}
