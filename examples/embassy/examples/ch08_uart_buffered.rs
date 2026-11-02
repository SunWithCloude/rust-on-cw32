//! 第 8 章例程 2：BufferedUart 回显 + 心跳 LED（双任务并发）。
//!
//! UART1（PA02=TXD / PA03=RXD，AF1，115200-8N1）跑 BufferedUart：
//! 接收进 64 字节环形缓冲（embassy-sync Pipe），echo 任务回显，
//! heartbeat 任务翻转 PC13（1 秒周期，作为"系统活着"的可观测信号）。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch08_uart_buffered -p cw32-book-embassy
//! 状态：同源例程（embassy-cw32 examples/buffered_echo）已实机验证
//! （2026-10-24：心跳 LED + ISER bit27 + 注入挂起全链路；数据回环待
//! USB 转串口实测，出处见第 8 章正文）。

#![no_std]
#![no_main]

use embassy_executor::{main, task};
use embassy_time::Timer;

use embassy_cw32::{
    Config, Peripherals,
    gpio::{Level, Output, Pin},
    uart::{self, BufferedUart},
};

embassy_cw32::bind_interrupts!(struct Irqs {
    UART1 => uart::Uart1Irq;
});

#[task]
async fn echo(
    mut rx: embassy_cw32::uart::BufferedUartRx,
    mut tx: embassy_cw32::uart::BufferedUartTx,
) {
    let mut buf = [0u8; 32];
    loop {
        let n = rx.read(&mut buf).await; // 挂起直到管道里至少 1 字节
        tx.write(&buf[..n]).await; // 写入发送管道，kick 后台排空
        tx.flush().await; // 等 TC：最后一个字节真正移位完毕
    }
}

#[main]
async fn main(spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default());

    let p = Peripherals::take().unwrap();
    embassy_cw32::sysctrl_enable_gpioc(p.sysctrl);
    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    let u = BufferedUart::new_uart1(
        p.uart1,
        Pin::new(p.gpioa, 2),
        Pin::new(p.gpioa, 3),
        1, // AF1
        Irqs,
        Default::default(), // uart::Config：115200
    );
    let (tx, rx) = u.split();
    spawner.spawn(echo(rx, tx).unwrap());

    loop {
        Timer::after_millis(1000).await;
        led.toggle();
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
