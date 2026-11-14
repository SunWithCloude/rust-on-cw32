# 第 9 章 bind_interrupts! 与中断绑定

这一章你会学到：

- `bind_interrupts!` 宏展开后到底是什么：强符号向量函数 + 编译期"绑定证明"；
- 为什么绑错中断是编译错误而不是运行时事故（两个真实实验的真实报错）；
- `IrqHandler`/`Binding` 两个类型与 trait 各自的角色；
- NVIC 使能发生在哪——以及生态仓 v0.1 那次"向量有了但中断进不来"的教训；
- 最小例程 `ch09_bind_uart.rs`。

第 8 章已经用过两次 `bind_interrupts!`：绑定 GPIOA 的端口中断、绑定 UART1。
这一章把它拆开，看清每个零件。

## 9.1 问题：中断向量怎么落到你的代码

硬件层面，Cortex-M0+ 复位后从向量表（`__INTERRUPTS`，由 cortex-m-rt 铺设）
取每个异常/中断的入口地址。CW32L012 有 32 个外设中断槽位，PAC 的 `rt`
feature 在构建脚本里为每个槽位生成一条弱别名（`cw32l012-pac-um`
`build.rs`）：

```text
PROVIDE(GPIOA = DefaultHandler);
PROVIDE(UART1 = DefaultHandler);
…（共 32 条，槽位号即 IRQ 号：GPIOA=5、GPIOB=6、GPIOC=7、GPIOF=8、
  UART1=27、UART2=28、UART3=29……）
```

`PROVIDE` 的语义是"如果没人定义这个符号，就用 `DefaultHandler` 兜底"。
所以绑中断的 C 风格做法是：**自己定义一个同名的强符号**把它盖上去：

```rust,ignore
#[unsafe(no_mangle)]
unsafe extern "C" fn UART1() {
    // 你的处理代码
}
```

这个名字来自向量表，写错一个字母（比如 `UART_1`）链接器**不会报错**——
弱别名仍在兜底，你的函数成了没人调用的死代码，中断静默落进 `DefaultHandler`
的死循环。第 2 章埋过伏笔：PAC 拉进链接的正是这张向量表骨架；当时没出事，
只因为例程还不需要外设中断。

`bind_interrupts!` 要解决的就是"名字对不对"从链接期悄悄溜走的问题。

## 9.2 宏展开：一条宏，两件事

```rust,ignore
embassy_cw32::bind_interrupts!(struct Irqs {
    UART1 => uart::Uart1Irq;
});
```

这行宏展开（`embassy-cw32/src/interrupt.rs`）生成两样东西：

**第一，向量函数**。为 `UART1` 生成强符号，覆盖 PAC 的弱别名：

```rust,ignore
#[unsafe(no_mangle)]
unsafe extern "C" fn UART1() {
    <Uart1Irq as IrqHandler<interrupt::UART1>>::call()
}
```

`uart::Uart1Irq` 是驱动提供的"每 IRQ 一个"的标记类型，它实现
`IrqHandler<UART1>` trait（`fn call()` 在中断上下文里执行驱动自己的派发
逻辑——第 8 章 `buffered_irq` 排空管道、`on_irq` 唤醒 waker，入口都在这）。
向量函数本身不写业务，只负责把控制权转交驱动的 handler 类型。

**第二，绑定证明**。给你的结构体实现标记 trait：

```rust,ignore
unsafe impl Binding<interrupt::UART1> for Irqs {}
```

`Binding<INT>` 是个空 trait（`unsafe` 是因为"结构体真的绑定了这个中断"
这个事实只有宏能保证）。它不携带任何数据，只把"Irqs 持有 UART1 中断"
写进类型系统——供下一步的构造函数检查。

两件事互为表里：向量函数保证中断进来后有人接，`Binding` 保证你想用外设时
必须先绑好向量。

## 9.3 构造函数的编译期校验

驱动的构造函数把证明写进了签名（`embassy-cw32/src/uart.rs`）：

```rust,ignore
pub fn new_uart1(
    regs: &'static crate::Uart,
    tx: Pin<impl GpioPort>,
    rx: Pin<impl GpioPort>,
    af: u8,
    _irqs: impl Binding<interrupt::UART1>,   // ← 校验点
    config: Config,
) -> Self
```

参数名带下划线（`_irqs`）——它运行期不被使用，存在的唯一意义就是让编译器
检查类型。**构造外设之前，必须先绑好它的中断**，否则编译不过。

绑错会怎么样？下面两个实验都是真编译的结果（`thumbv6m-none-eabi`
release，rustc 报错原文摘录）。实验文件本身不进 `examples/`——它们的
存在意义就是编译失败。

**实验 1：中断名写对了，handler 类型张冠李戴**——把 GPIO 的 handler 绑到
UART1 的中断名上：

```rust,ignore
embassy_cw32::bind_interrupts!(struct Irqs {
    UART1 => embassy_cw32::gpio::GpioAIrq;   // 绑错：这是 GPIOA 的 handler
});
```

真实报错：

```text
error[E0277]: the trait bound `GpioAIrq: IrqHandler<UART1>` is not satisfied
   --> examples/embassy/examples/zz_exp1_wrong_handler.rs:16:14
    |
16  |     UART1 => embassy_cw32::gpio::GpioAIrq;
    |              ^^^^^^^^^^^^^^^^^^^^^^^^^^^^ the trait `IrqHandler<UART1>` is not implemented for `GpioAIrq`
    |
help: the trait `IrqHandler<UART1>` is not implemented for `GpioAIrq`
      but trait `IrqHandler<GPIOA>` is implemented for it
    = help: for that trait implementation, expected `GPIOA`, found `UART1`
```

宏在展开处就把错误拦下：`GpioAIrq` 只实现 `IrqHandler<GPIOA>`，冒充不了
UART1 的 handler。对比 C 风格——链接器对这种错一声不吭。

**实验 2：绑定本身合法，但构造函数要的证明没有**——GPIOA 绑得端端正正，
却拿去构造 UART：

```rust,ignore
embassy_cw32::bind_interrupts!(struct Irqs {
    GPIOA => gpio::GpioAIrq;   // 合法，但与 UART 无关
});
// …
let mut u = Uart::new_uart1(p.uart1, tx, rx, 1, Irqs, Default::default());
```

真实报错：

```text
error[E0277]: the trait bound `Irqs: Binding<UART1>` is not satisfied
   --> examples/embassy/examples/zz_exp2_missing_binding.rs:28:9
    |
23  |     let mut u = Uart::new_uart1(
    |                 --------------- required by a bound introduced by this call
28  |         Irqs,
    |         ^^^^ unsatisfied trait bound
    |
help: the trait `Binding<UART1>` is not implemented for `Irqs`
      but trait `Binding<GPIOA>` is implemented for it
note: required by a bound in `embassy_cw32::uart::Uart::new_uart1`
324  |         _irqs: impl Binding<interrupt::UART1>,
    |                     ^^^^^^^^^^^^^^^^^^^^^^^^^ required by this bound
```

报错甚至替你想好了下一步：给 `Irqs` 补上 `UART1 => uart::Uart1Irq;`。

两个实验合起来是同一句话：**中断号与 handler 类型在编译期就绑死了**——
绑错中断的固件根本无法存在。

## 9.4 NVIC 使能发生在哪

`bind_interrupts!` 只生成向量与证明，**不碰 NVIC**。使能发生在驱动构造器
（以及 GPIO 的 `wait_edge`）内部：`nvic_enable::<I>()` 直接写 ISER
（0xE000_E100，`1 << I::NUMBER`，IRQ27 即 bit27）。

这个分工曾出过一次真实事故，值得引以为鉴（embassy-cw32 CHANGELOG 0.2.0
"修复"条目）：v0.1 的全部异步驱动——向量函数有了、`Binding` 也有了——
**但谁都没写 ISER**，中断根本进不来。漏洞长期没暴露的原因很有意思：blink
例程的定时依赖走的是 SysTick，那是内核异常，不经过 NVIC 的外设中断线；
直到移植缓冲式 UART 时中断驱动的路径才第一次被真正踩到。修复后
`Uart`/`BufferedUart`/`Spi`/`I2c` 的构造与 GPIO 的 `wait_edge` 都会自动
使能对应 NVIC 线——上板证据是第 8 章引过的 `buffered_echo` 记录：
`ISER bit27 = 1`。

设计上"构造即使能"是合理的：拿到驱动对象的时刻，就是它开始产生中断的
时刻。需要留意的只有一点——未绑定的 IRQ 落进 `DefaultHandler` 死循环，
它是"谁忘了绑中断"的运行期墓碑；而绑定了却没被任何驱动使用的 IRQ，
向量函数存在、NVIC 没人开，无副作用。

## 9.5 例程：ch09_bind_uart.rs

最小闭环：一份 `bind_interrupts!` + 逐字节 `Uart` 回显（UART1，
PA02/PA03 AF1，115200-8N1）。

```rust
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
```

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch09_bind_uart -p cw32-book-embassy
```

回显主体是第 8 章"逐字节 `Uart`"那一列的最小实现：`read_byte().await`
在 RC 上挂起（等待期间才打开 IER 的 RC 位，空闲无中断风暴），收到字节后
`write_byte().await` 等 TXBE 写 TDR。`Err(_e) => {}` 不是吞错误——
`read_byte` 返回错误前已经用 ICR 清掉了错误标志（ICR 是 R1W0，写 0 清除，
手册 §21 UART 寄存器组），这里继续收下一个字节即可；要区分错误种类的话，
`UartError` 的四个变体（Overrun/Framing/Parity/Noise）随手可 match。

与 `BufferedUart` 版本的分工见 8.4 的对比表：这个例程演示"逐字节"形态，
两种形态共用同一套 `bind_interrupts!` 与构造器校验。

## 9.6 板上验证

本例程与生态仓 `uart_echo_async` 代码同源（仅注释不同），后者在
embassy-cw32 README "Verification status" 中的记录是（2026-10-24）：
**已烧录上板、可运行、无硬件故障；异步回环的实际收发当时还没有 USB 转
串口适配器来测量**。因此本例程标注为：**同源已验证（有限）**——上板跑
通有记录，完整数据回环待实测。首次完整上板核对点：

- 适配器交叉接线（适配器 RXD 接板 PA02/TXD）、115200-8N1；
- 逐字节回显：发送任意序列，回显逐字节一致；
- 故意发一个波特率不匹配的字节流制造帧错误，确认固件不死机
  （`Err` 分支继续收字节）。

## 9.7 常见坑

- **绑了中断却忘传 `Irqs`**（或传了另一个结构体）：E0277，
  `Binding<UART1>` 不满足——照 9.3 实验 2 的报错补即可。
- **同一中断绑两次**：一个中断在一套 `Binding` 里只能出现一次；写两遍
  会生成重复的向量符号与重复的 trait 实现，编译直接失败。
- **自己再写一个 `#[unsafe(no_mangle)] fn UART1`**：与宏生成的强符号
  重复定义，链接错误。中断逻辑写进驱动的 handler 类型，或换用你自己的
  `IrqHandler` 实现——不要另立向量符号。
- **绑定了却没使能的误解**：NVIC 使能在构造器/`wait_edge` 里自动完成，
  不需要（也不应该）手写 ISER；想手写，先确认你绕开的到底是什么。
- **共享向量家族**：SPI2/3 共享 `SPI23` 中断，L083 的 UART1 与 UART4
  共享向量（typelevel 名 `UART1_UART4`）。L012 上用不到，跨家族移植时
  见附录 A 的家族矩阵。
