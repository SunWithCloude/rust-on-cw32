# 第 8 章 异步 GPIO 与串口

这一章你会学到：

- `Input` 的 `wait_for_*` 等待家族，以及"每端口一个中断"的按位唤醒机制；
- `BufferedUart` 的生产者/消费者结构：IRQ 排空 RDR 进管道，任务从管道取数；
- 逐字节 `Uart` 与 `BufferedUart` 的取舍（CPU 占用 / 抗突发 / 吞吐 / RAM）；
- 双任务并发：echo 任务 + 心跳任务互不阻塞地跑在同一执行器上；
- 两个例程的构建与板上验证状态。

两个例程对应两个文件：`ch08_gpio_wait.rs`（按键等沿 + LED 翻转）与
`ch08_uart_buffered.rs`（缓冲串口回显 + 心跳）。

## 8.1 异步输入：等一个边沿

第二部分扫按键的方式是循环轮询 `get_level()`。embassy-cw32 的 `Input`
提供另一种姿势——把任务挂起，边沿到来时由中断唤醒：

```rust,ignore
let mut btn = Input::new(Pin::new(p.gpioa, 0), Pull::Up);
btn.wait_for_falling_edge().await; // 到这里为止 CPU 都在别处（或睡着）
```

等待家族共五个方法，全部出自 `embassy-cw32/src/gpio.rs`：

- `wait_for_rising_edge()` / `wait_for_falling_edge()` / `wait_for_any_edge()`
  ：等待上升沿 / 下降沿 / 任一边沿；
- `wait_for_high()` / `wait_for_low()`：等电平。实现是"边沿触发 + 电平复查"
  ——先查 IDR，电平不符就等任一边沿再来复查，直到目标电平出现。

`Input::new(pin, pull)` 把引脚配成数字输入（DIR=1、ANALOG=0）并设置上/下拉
。下拉有个硬件事实要记住：**L012 上只有 PF3 有真实下拉电阻**（手册
§9.6.3），其余引脚 `Pull::Down` 等同 `Pull::None`——所以"按键接 GND"的
标准接法配 `Pull::Up`。

**每端口一个中断**。CW32 的 GPIO 中断按端口共享：GPIOA/B/C/F 各占一个
IRQ（IRQ5/6/7/8，手册中断表）。五个等待 API 内部的公共实现（`wait_edge`）
做这几件事：

1. 按端口槽位使能 NVIC 中断线（`nvic_enable::<GPIOA>()` 等，写 ISER）；
2. 清残留中断标志与软件触发位图，然后置该引脚的上升/下降沿使能
   （`GPIOx_RISEIE`/`FALLIE`）；
3. 注册 per-pin waker 挂起任务。端口 IRQ 到来时：把 ISR 里的 pending 位
   并进软件触发位图、写 ICR 清硬件标志、逐位唤醒等待的 waker；
4. 等到边沿后，自动关掉该脚的边沿使能——不留后台中断。

与 UART 不同，GPIO 的 NVIC 使能与派发都在驱动里就位了，**但向量函数必须
由你绑定**：不给 GPIOA 调 `bind_interrupts!`，中断会落进 PAC `device.x`
预留的弱默认 `DefaultHandler`（死循环）——任务从此永远挂起，没有报错、
没有日志。这正是第 9 章整章要讲的东西。

## 8.2 例程 1：ch08_gpio_wait.rs

按键（或杜邦线）接在 PA0 与 GND 之间，板上 LED 在 PC13。按下 = PA0 出现
下降沿 = LED 翻转一次。

```rust
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

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
```

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch08_gpio_wait -p cw32-book-embassy
```

三个值得停留的细节：

**AHBEN 一次写两个位**。这个例程要同时用 GPIOA（按键）和 GPIOC（LED），
而第 7 章讲过 AHBEN 是键控整字写——`sysctrl_enable_gpioa()` 和
`sysctrl_enable_gpioc()` 各自整字覆盖，连着调第二个会把第一个清掉。所以
这里直接对寄存器一次写齐：`SYSCTRL_KEY | (1 << 4) | (1 << 6)`
（GPIOA=bit4、GPIOC=bit6，手册 §4.7）。

**只绑被等待的端口**。`bind_interrupts!` 只写了 GPIOA：LED 是 `Output`，
不产生中断；GPIOC 即使绑了也用不上。宏展开会生成一个强符号向量函数
`GPIOA()`（IRQ5 的入口），第 9 章逐行拆解。

**消抖**。机械按键的按下/松开伴随几毫秒的抖动，一次按压可能触发多个
下降沿、LED 连翻几次。异步写法下的经典消抖是"等到边沿后再
`Timer::after_millis(50).await` 确认电平"，留给读者作为练习改造本例程。

## 8.3 例程 2：ch08_uart_buffered.rs

`BufferedUart` 是带后台缓冲的 UART 驱动（设计吸收自 fan2nd/cw32-hal2，
embassy-sync `Pipe` 做环形缓冲，收/发各 64 字节）。数据流是两条单向管道：

- **接收**：UART1 中断常开（RC + 全部错误位）。字节到达 → IRQ 把 RDR
  逐字节排空进 64 字节的 RX 管道（满了就丢弃并计入诊断计数器）；任务侧
  `rx.read(&mut buf).await` 从管道取数，挂起直到至少 1 字节。
- **发送**：`tx.write(&buf[..n]).await` 把数据写进 64 字节的 TX 管道即
  返回（管道满才挂起），同时 kick 一次后台排空——IRQ 在 TXBE 允许时把
  管道内容写进 TDR（写前清 TC，防陈旧标志把 flush 提前唤醒）；`flush()`
  等"管道空 + TC 置位"，即最后一个字节真正移位完毕。

例程把 echo 与心跳跑成两个并发任务：echo 任务挂在 `rx.read` 上，心跳任务
每秒翻转 PC13——如果 LED 还在闪，说明执行器活着、echo 只是停在管道读上。

```rust
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
```

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch08_uart_buffered -p cw32-book-embassy
```

引脚按用户板口径：UART1 = PA02(TXD)/PA03(RXD)，复用功能 AF1（手册表 9-2；
实测口径见第 5 章）。`Default::default()` 是 `uart::Config`，默认波特率
115200-8N1。

`rx.read(&mut buf)` 返回的是 `usize` 而不是 `Result`——这是 `BufferedUartRx`
的固有方法（阻塞到至少 1 字节，返回实际读到的个数）。之所以能"无错误返回"，
是因为接收错误在 IRQ 层已经被处置：坏字节直接丢弃并计数，不进管道。诊断
接口有两个：`rx.ready()` 看管道里现有多少字节，`rx_dropped()` 看累计丢了
多少。

## 8.4 逐字节 Uart 与 BufferedUart 的取舍

embassy-cw32 的 UART 有两种任务侧形态（L012 还有第三种 DMA 路径，语义级
实现待实板验证，第 12 章再讲；三种方式在同一个实例上互斥）：

| | 逐字节 `Uart`（第 9 章例程） | `BufferedUart`（本章例程 2） |
|---|---|---|
| 接收路径 | IRQ 唤醒任务，任务自己读 RDR（1 字节） | IRQ 把 RDR 排空进 64 字节管道，任务读管道 |
| 抗突发 | 无缓冲：任务必须在下一个字节到来前读走（115200 下约 87µs），否则溢出（ORE） | 任务可以迟到约 5.6ms（64 字节 @115200）不丢数据 |
| 发送节奏 | `write_byte` 等 TXBE，逐字节推进 | `write` 进管道即返回，IRQ 接管发送节奏，`flush` 显式等 TC |
| 多任务 | `split` 后收发各一个任务 | 收发半部可分属不同任务；读端基于 `Pipe`，天然支持多任务并发读 |
| 错误上报 | `read_byte` 返回 `UartError`（ORE/FE/PE/NE，按此优先级） | 错误字节在 IRQ 层丢弃并计数（`rx_dropped()`），不上报任务 |
| RAM 代价 | 无静态缓冲 | 每实例 RX+TX 各 64 字节（3 个实例的管道是全局静态，共 384 字节） |

选择标准一句话：**命令式、低速率、要逐字节看错误的交互用逐字节 `Uart`；
要并发、要抗突发、要吞吐平稳（日志输出、协议流）用 `BufferedUart`**。
逐字节形态还有一个隐含要求：主任务必须把大部分时间花在"等串口"上——
它被唤醒后若去干了 100µs 的别的事，下一个字节就可能溢出。`BufferedUart`
把这份时效压力从任务转移给了中断（中断只要在 1 字节时间内排空 RDR 即可）。

## 8.5 构建与板上验证

例程 2 标注**同源已验证**，出处是 embassy-cw32 README "Verification
status" 的 2026-10-24 记录——`buffered_echo`（本例程的直系同源）在
CW32L012 板上：

- 心跳 LED 持续翻转，证明执行器带着两个并发任务在跑（echo 任务停在管道
  读上）；
- `ISER bit27 = 1`（UART1 的 NVIC 线），证明 v0.2 的"构造器自动使能 NVIC"
  修复生效；
- 经调试探针向 `ISPR` 注入一个 UART1 挂起，中断正常走完
  向量 → `bind_interrupts!` → `buffered_irq` → critical-section 全链路，
  `ICPR` 清除、无锁死；
- **如实标注**：实际的缓冲数据回环（PC 发什么、板回什么）当时没有 USB
  转串口适配器，未实测。首次完整上板请核对：适配器交叉接线（适配器 RXD
  接板 PA02）、115200-8N1、发送任意字节得到逐字节回显。

例程 1（`ch08_gpio_wait`）标注**仅编译未上板**。首次上板核对点：

- 板上若没有按键，用杜邦线把 PA0 碰一下 GND，每碰一次 LED 翻转一次；
- 观察一次"按下"是否对应多次翻转（抖动，见 8.2 的消抖练习）；
- 换引脚时三处要一致：`Input::new` 的引脚号、`bind_interrupts!` 的端口名、
  AHBEN 位（GPIOA/B/C = bit4/5/6）。

## 8.6 常见坑

- **忘了绑端口中断**：`wait_for_*` 永远挂起——向量落在弱默认
  `DefaultHandler`（死循环），无报错。等待哪个端口，就绑哪个端口。
- **端口时钟没开（或被覆盖）**：`Input`/`Output` 写寄存器前先确认 AHBEN
  位已置。两个端口必须一次键控写写齐，分两次写会互相覆盖。
- **按键抖动**：一次按压多次翻转不是 bug，是物理。加 50ms 电平确认。
- **串口无回显**：先查 AF1 与交叉线（板的 TXD 接适配器 RXD），再查
  `bind_interrupts!` 是否绑了 UART1。
- **以为 BufferedUart 不丢数据**：64 字节管道是硬上界；RX 满后新字节被
  丢弃并计入 `rx_dropped()`。大批量传输要自己加流控，或改走 DMA（第 12 章）。
- **混用三种传输方式**：逐字节 / Buffered 管道 / DMA 在同一 UART 实例上
  互斥，混用前先让上一方式收尾（`uart` 模块文档）。
