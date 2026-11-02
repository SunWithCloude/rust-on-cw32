//! 第 9 章例程：bind_interrupts! 的最小 UART 用法（逐字节回显）。
//!
//! UART1（PA02=TXD / PA03=RXD，AF1，115200-8N1），逐字节 Uart：
//! read_byte 等 RC、write_byte 等 TXBE，等待期间任务挂起。构造函数
//! 要求传入 `impl Binding<UART1>` 的证明——Irqs 由 bind_interrupts!
//! 生成，绑错中断（比如把 UART1 写成 GPIOA）直接编译失败（第 9 章
//! 有该实验的真实报错）。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch09_bind_uart -p cw32-book-embassy
//! 状态：同源例程（embassy-cw32 examples/uart_echo_async）已上板：
//! 可运行、无硬错误（README 验证记录）；完整收发回环待 USB 转串口
//! 实测，出处与核对点见第 9 章正文。

#![no_std]
#![no_main]

use embassy_executor::main;

use cw32_book_embassy as _; // 继承 lib.rs 的 panic handler（panic-probe 同款机制）
use embassy_cw32::{
    Config, Peripherals,
    gpio::Pin,
    uart::{self, Uart},
};

// 把 UART1 中断（IRQ27）绑到驱动的 Uart1Irq 处理类型：宏展开出
// 强符号向量函数 UART1（覆盖 PAC device.x 里的弱默认），并给 Irqs
// 实现 Binding<UART1>
embassy_cw32::bind_interrupts!(struct Irqs {
    UART1 => uart::Uart1Irq;
});

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default());

    let p = Peripherals::take().unwrap();

    // 构造即校验：Irqs 必须实现 Binding<interrupt::UART1>，否则编译
    // 失败。NVIC 中断线（ISER bit27）由构造函数自动使能
    let mut u = Uart::new_uart1(
        p.uart1,
        Pin::new(p.gpioa, 2),
        Pin::new(p.gpioa, 3),
        1, // AF1
        Irqs,
        Default::default(),
    );

    loop {
        match u.read_byte().await {
            Ok(b) => {
                u.write_byte(b).await;
            }
            Err(_e) => {} // 错误标志已在 read_byte 内经 ICR 清除，继续下一字节
        }
    }
}
