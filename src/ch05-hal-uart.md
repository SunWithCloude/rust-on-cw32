# 第 5 章 串口收发

LED 只会闪，串口才会说话。这一章把 UART1 跑起来：复位后打一行
欢迎词，然后把你发来的每个字节原样回显——这是所有后续调试手段
的地基。

**这一章你会学到：**

- UART1 引脚怎么选：PA02(TX)/PA03(RX) 走 AF1（用户板实测口径）；
- 波特率寄存器 `BRRI`/`BRRF` 的分频公式，以及 4 MHz 下 115200 的误差从哪来；
- `Uart` 的阻塞收发 API 与接收错误自愈（ORE/NE/PE/FE）；
- "波特率错一位""引脚 AF 没切"这些故障各自长什么样。

## 5.1 引脚与复用：先看表 9-2 再接线

CW32L012 有 UART1/2/3 三个串口，每个都有多组引脚可选（手册表 9-2；
一个 UART 可有多组引脚，但同一物理引脚同一时刻只能归属一个 UART）：

| UART | 组合 A | 组合 B |
|------|--------|--------|
| UART1 | **PA02=TXD / PA03=RXD（AF1）** | PA09=TXD / PA10=RXD（AF1） |
| UART2 | PA02=TXD / PA03=RXD（AF2） | PA07=TXD / PA06=RXD（AF1） |
| UART3 | PA04=TXD / PA05=RXD（AF1） | PA01=TXD / PA00=RXD（AF2） |

本书与 `cw32l012-hal` 的例程统一用 **UART1 组合 A：PA02(TX)/PA03(RX)，
AF1**——这是用户板的实测口径。注意 PA02/PA03 这对物理引脚同时挂在
UART1（AF1）和 UART2（AF2）名下：切错 AF 编号，寄存器一切正常，
引脚上却是 UART2 在说话，示波器有波形、串口助手全是乱码。

引脚配置复用第 4 章的类型状态 `Pin`，TX 走复用输出、RX 走复用输入：

```rust,ignore
let tx = Pin::new(p.gpioa, PortId::A, 2, &mut rcc).into_af_output(Af::Af1);
let rx = Pin::new(p.gpioa, PortId::A, 3, &mut rcc).into_af_input(Af::Af1);
```

`into_af_input` 在切 AF 的同时会自动使能内部上拉——这是手册表 21-1
推荐的标准接法（数字功能、上拉输入、复用），对端没接或总线空闲时
避免引脚悬空采进噪声帧。

## 5.2 波特率：BRRI × 16 + BRRF

UART 的波特率时钟 UCLK 默认取自 PCLK（`CR1.SOURCE=00`），16 倍采样
（`CR1.OVER=00`），分频公式（手册 §21）：

```text
Baud = UCLK / (BRRI × 16 + BRRF)
```

`BRRI` 是整数部分（16 位），`BRRF` 是小数部分（4 位），两者分开写进
两个寄存器。HAL 的 `calc_brr` 按四舍五入求 `div = round(UCLK/Baud)`，
再拆成 `BRRI = div/16`、`BRRF = div%16`。几个有单元测试背书的数值：

| PCLK | 目标波特 | BRRI | BRRF | 实际波特 | 误差 |
|------|----------|------|------|----------|------|
| 4 MHz | 115200 | 2 | 3 | 114286 | −0.79% |
| 96 MHz | 115200 | 52 | 1 | 115246 | +0.04% |
| 4 MHz | 250000 | 1 | 0 | 250000 | 0（整除） |
| 4 MHz | 300000 | — | — | `calc_brr` 返回 Err → panic | |

看第一行：4 MHz ÷ 115200 = 34.7，四舍五入成 35，实际波特率 114286，
差 −0.79%。异步串口容许两端各 ±2% 左右的偏差，−0.79% 安全；但如果
你以为 PCLK 是 4 MHz 而实际固件跑在别的频率（比如忘了自己改过
分频），误差立刻冲出容忍窗——**波特率错一位的表现**通常是两种之一：
全字符乱码（分频差得多），或偶发帧错误（分频差得少，采样点蹭到
边沿）。`Uart::new` 对不可实现的波特率（`PCLK/16 < baud` 时 `BRRI`
为 0）直接 panic，宁可启动期就炸也不悄悄跑错。

## 5.3 Uart 的构造与收发 API

`Uart::new` 按传入寄存器块的基地址自动识别是 UART1/2/3，并自动开
对应 APB1 时钟门控（UART1 = `APBEN1[3]`，写保护键细节同第 4 章），
然后停收发 → 写 `BRRI`/`BRRF` → 清标志 → 使能收发，固定 8-N-1：

```rust,ignore
let mut uart = Uart::new(p.uart1, tx, rx, &mut rcc, clocks.pclk_hz, 115_200);
```

注意第 4、5 个参数：`clocks.pclk_hz` 是第 4 章 `clocks()` 快照里的
PCLK，公式用它做 UCLK——**传错频率，波特率必然错**。

收发 API 一览（全部阻塞或轮询，无中断、无队列）：

| 方法 | 行为 |
|------|------|
| `write_byte(b)` | 等 `ISR.TXBE`（发送空）后写 `TDR` |
| `write_bytes(&[u8])` / `write_str(&str)` | 逐字节阻塞发送 |
| `flush()` | 等 `ISR.TC`（含停止位全部移出） |
| `read_byte()` | 阻塞等 `ISR.RC`，读 `RDR` 取字节 |
| `try_read_byte()` | 无数据返回 `None`，主循环轮询友好 |
| `take_error()` | 解码并清除接收错误，按溢出>帧错>校验>噪声取优先级 |

接收路径有个值得留意的自愈设计：`read_byte`/`try_read_byte` 发现
`ISR` 挂着 ORE（溢出）/NE/PE/FE 任一错误标志时，立即清标志并丢弃
`RDR` 里的残留数据。不清 ORE 的后果是——溢出标志一旦挂起不清，
部分实现的接收路径就再也不置 `RC`，表现为"串口一开始能收，塞爆
一次后永远装死"。HAL 把这类"溢出假死"在驱动层挡掉了。

## 5.4 例程：ch05_uart_echo.rs

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch05_uart_echo -p cw32-book-hal
```

完整例程（`examples/hal/examples/ch05_uart_echo.rs`）：

```rust
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

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
```

接线（USB-TTL 串口适配器，**TX/RX 交叉**）：

```text
MCU PA02 (TXD) ──────> 适配器 RXD
MCU PA03 (RXD) <────── 适配器 TXD
GND           ───────  GND（必须共地）
```

烧录后串口助手以 115200-8-N-1 打开对应 COM 口：先收到一行
`CW32L012 Rust HAL ready`，随后键入什么就回显什么。回显循环用
`try_read_byte` 轮询而不是 `read_byte` 阻塞，行为上等价——本例
主循环里没有别的事可做，你完全可以把 `if let` 换成 `read_byte`
体会两种风格的差别。

## 5.5 板上验证

**状态：仅编译未上板。** 如实交代依据：

- cw32l012-hal 的 `TESTING.md` "实测通过"表中**没有** uart_echo 条目；
  其 README 的"未验证"清单明确列着"UART 收发数据路径"，路线图里
  "UART 收发上板验证（uart_echo 对夹）"也未勾选。
- 已有的旁证：`TESTING.md` 2026-10-24 的 ecosystem 条目（embedded-io
  写路径实测）确认 `IO_WRITTEN=13`——`Uart` 底层的阻塞写字节路径
  确实在 PA02 上发出了 13 字节并观察到波形；但**完整回显链路
  （含 RX）尚未用 USB-TTL 对夹验证过**。

首次上板请按顺序核对：

1. 只看不发：上电应先收到欢迎行——验证 TX 路径、波特率、COM 口设置；
2. 收不到欢迎行时：先查 TX/RX 是否交叉接反、是否共地，再查串口助手
   波特率是否 115200；
3. 有欢迎行但回显错字：RX 路径问题，重点查 PA03 的 AF1 是否配置、
   适配器 TXD 是否真的接到了 PA03；
4. 全程无波形：用示波器/逻辑分析仪看 PA02，无波形则回头查 AF 切换
   与 APBEN1 门控。

## 5.6 常见坑

- **引脚 AF 没切**：`Uart::new` 只管 UART 外设本身，引脚必须先行
  `into_af_output/into_af_input(Af::Af1)`。AF 停留在 0（GPIO）时
  外设一切"正常"，引脚上没有信号。
- **AF 编号切错**：PA02/PA03 挂着 UART1(AF1) 和 UART2(AF2) 两套功能，
  外设与 AF 编号必须配套，混搭即乱码。
- **时钟门控**：UART 时钟在 APBEN1（键值保护同 AHBEN）。`Uart::new`
  已自动处理；绕过 HAL 手写寄存器时别忘了键值 0x5A5A。
- **pclk_hz 传错**：`Uart::new` 的第五参数是"你以为的 PCLK"，
  与实际频率不符时波特率按比例漂移——常见于改过主频却忘了同步。
- **只发不收或"装死"**：老代码自己清错误标志时漏了 ORE，接收路径
  会被溢出标志卡死；用 HAL 的 `read_byte`/`try_read_byte` 则自动清。
- **接线不交叉 / 不共地**：TX 对 TX 永远收不到；少了 GND 参考
  电平，表现为偶发帧错误或一阵一阵的乱码。
