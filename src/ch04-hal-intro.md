# 第 4 章 认识 cw32l012-hal

第 2 章里，点亮一颗 LED 需要手写五步寄存器操作；这一章开始换成
**阻塞式 HAL** `cw32l012-hal`——同样的闪烁，主代码只剩五行。

**这一章你会学到：**

- `cw32l012-hal` 的设计取舍：零外部依赖、自带运行时、panic handler 为什么不在 HAL 里、本书如何把它收进例程 crate 的 lib.rs；
- `Peripherals::take()` 的"一次语义"与外设所有权模型；
- 逐段精读 `ch04_blink_pc13.rs`：时钟快照、GPIO 门控的 `0x5A5A` 保护写、
  `into_output` 到底清了哪些复位残留；
- `Delay` 的计时基准从哪里来，换频之后为什么会失准。

## 4.1 crate 定位与设计哲学

`cw32l012-hal` 是武汉芯源 CW32L012 的 `no_std` 硬件抽象层，寄存器层
（`pac` 模块）内置于 crate 之中，不依赖外部 PAC。它有几个明确的取舍：

**零外部依赖起步。** 不开任何 feature 时，crate 的依赖列表是空的——
`Cargo.toml` 里没有 cortex-m、没有 cortex-m-rt。寄存器原语（`Rw`/`Ro`/`Wo`）、
向量表、复位初始化全部自己实现。代价是不能复用生态里现成的部件，
好处是构建树完全可控、每一行代码都能对照手册。

**自带运行时。** 默认 feature `runtime` 提供两样 cortex-m-rt 的替代品：

1. 48 项中断向量表（弱符号可覆盖，IRQ0..31 与手册表 5-1 一致）；
2. 复位入口 `Reset`：完成 `.data` 段从 Flash 到 SRAM 的复制和 `.bss` 清零，
   然后调用你的 `main`。

所以例程**不需要** `cortex-m-rt`，也没有 `cortex_m_rt::entry` 宏——直接写
`fn main() -> !` 即可。

**panic handler 不由 HAL 提供。** `#[panic_handler]` 在整个固件里必须
唯一，HAL 不替你做这个决定——这是应用侧的选择。本书把这个决定只做
一次：handler 写在例程 crate（`cw32-book-hal`）的 `lib.rs` 里，各例程
一行 `use cw32_book_hal as _;` 把 lib 拉进链接、自动继承——与第 3 章
`use panic_probe as _;` 同一机制（panic-probe 的全部内容也不过是一个
panic handler）。handler 本身仍是最简形式：死循环。想接 RTT 打印
panic 信息的话，把第 3 章的 rprintln 搬进 lib 即可。

**feature 面一览：**

| Feature | 默认 | 作用 |
|---|---|---|
| `runtime` | ✅ | 向量表、`.data`/`.bss` 初始化、`interrupt!` 宏、NVIC 操作 |
| `ehal-1` | ✅ | `embedded-hal 1.0` trait（DelayNs / digital / SPI / I2C / PWM），对接生态驱动 |
| `critical-section-impl` | — | 基于 PRIMASK 的临界区实现（一个固件只能有一个实现） |
| `embedded-io` | — | `Uart` 的 `embedded_io::Read` / `Write` |
| `defmt` | — | 错误类型的 `defmt::Format` |

全部关掉（`default-features = false`）就回到零依赖的裸寄存器形态。

**版本口径**：crates.io 已发布 **0.9.2**，本书例程直接用这个版本
（`examples/hal/Cargo.toml`：`cw32l012-hal = "0.9.2"`）。0.9 主要是补
生态对接与实机验证，不改变既有签名。

## 4.2 所有权模型：Peripherals::take()

HAL 把全部外设寄存器块收进一个 `Peripherals` 结构体：

```rust,ignore
pub struct Peripherals {
    pub sysctrl: &'static pac::Sysctrl,
    pub gpioa: &'static pac::Gpio,
    pub uart1: &'static pac::Uart,
    // ……共 39 个字段，对应全外设基地址表
}
```

`Peripherals::take()` 只能成功一次——第一次返回 `Some(p)`，之后再调用
返回 `None`。这是嵌入式 Rust 的标准"单例"手法：编译期类型系统之外，
再给"同一个外设寄存器块只有一个所有者"上一道运行期保险。本书例程
统一用 `match` 展开它：

```rust,ignore
let p = match Peripherals::take() {
    Some(p) => p,
    None => loop { core::hint::spin_loop(); },
};
```

第二次拿到 `None` 只可能发生在代码路径出 bug 时（比如 `main` 被
递归调用），死循环即显式暴露问题。中断服务程序里需要再碰寄存器时，
用 `unsafe` 的 `Peripherals::steal()`——所有权冲突由写 ISR 的人自己负责
（第 6 章的定时器中断会用到）。

## 4.3 例程：ch04_blink_pc13.rs

构建命令（在本书仓库根目录执行）：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch04_blink_pc13 -p cw32-book-hal
```

完整例程如下（`examples/hal/examples/ch04_blink_pc13.rs`）：

```rust
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

use cw32_book_hal as _; // 继承 lib.rs 的 panic handler（panic-probe 同款机制）
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
```

`#[unsafe(no_mangle)]` 是 Rust 2024 edition 的要求：把 `main` 以裸符号
`main` 导出（运行时的 `extern "Rust" { fn main() }` 要按名字找到它），
`unsafe(...)` 块由作者为"符号名不被改写"这一前提担保。第 2 章的 PAC
例程已经用过同样写法。

下面逐段拆解这个例程里的五条关键路径。

### 段 1：时钟快照——Rcc::new 与 clocks()

```rust,ignore
let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
let clocks = rcc.clocks();
```

`Rcc::new` 拿到 SYSCTRL 和 FLASHCTRL 两个寄存器块，做三件事：登记
复位默认频率（SysClk = HSI = HSIOSC/24 = **4 MHz**，HCLK = PCLK = 4 MHz，
HSI 分频编码见手册 §4.7.4）、从 FLASH 工厂校准区加载 HSIOSC TRIM
（手册 §4.4.2，把 ±5% 的 RC 精度提到出厂校准级）。本例不变频，
`clocks()` 只是取出 `{ sysclk_hz, hclk_hz, pclk_hz }` 三个 Hz 数的快照，
后面波特率（第 5 章）和延时（本段末）都以此为基准。

时钟树长这样（加粗即复位默认路径）：

```text
HSIOSC 96MHz ──/24──> HSI 4MHz ──┐
HSE 晶体 ─────────────────────────┤
LSI ~32.8kHz ─────────────────────┼─> SysClk ──/1──> HCLK ──/1──> PCLK
LSE 32.768kHz ────────────────────┘
```

### 段 2：Pin::new 与 0x5A5A 保护写

```rust,ignore
let mut led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);
```

`Pin::new(p.gpioc, PortId::C, 13, &mut rcc)` 第一件事是给 GPIOC 开时钟：
`rcc.enable_gpio(PortId::C)`。CW32L012 的外设时钟门控寄存器（`AHBEN`/
`APBEN1`/`APBEN2`，手册 §4.7.11–4.7.13）带写保护——高 16 位必须写入
键值 **0x5A5A**，写保护键，低 16 位才是使能位。HAL 把这个细节收进
`Rcc` 的私有助手（摘自 cw32l012-hal `src/rcc.rs`）：

```rust,ignore
/// 置位（KEY 保护寄存器；KEY 位读为 0，低 16 位为状态）
fn set_bits(&self, reg: &pac::Rw<u32>, bits: u32) {
    let v = reg.read() & 0xFFFF;
    reg.write(sc::KEY | v | bits);
}
```

读回时只取低 16 位（KEY 位读为 0），写回时或上键值——调用方完全
感觉不到保护键的存在。忘了 KEY 会怎样？写入被硬件静默丢弃，GPIO
寄存器读了全是 0，LED 纹丝不动，且没有任何错误报告。这是 CW32
家族跟 STM32 差异最大的细节之一（STM32 的 RCC 不需要键值）。

### 段 3：into_output 清了哪些复位残留

`into_output(Level::Low)` 依次做四步（摘自 cw32l012-hal `src/gpio.rs`，
方向约定见手册 §9.6.1——**DIR=0 是输出，与 STM32 相反**）：

```rust,ignore
pub fn into_output(self, initial: Level) -> Output<'p> {
    let m = self.mask();
    match initial {
        Level::High => self.regs.bsrr.write(m),
        Level::Low => self.regs.bsrr.write(m << 16),
    }
    self.clear_extras();
    self.regs.dir.modify(|v| v & !m); // 0 = 输出
    Output { pin: self }
}
```

先经 BSRR 原子写好初始电平再切方向，切换瞬间不会冒出毛刺；随后
`clear_extras()` 把四样"复位残留"统一清零：

```rust,ignore
fn clear_extras(&self) {
    let m = self.mask();
    self.regs.opendrain.modify(|v| v & !m);
    self.regs.pur.modify(|v| v & !m);
    self.regs.pdr.modify(|v| v & !m);
    self.regs.analog.modify(|v| v & !m);
    self.set_af(0);
}
```

开漏、上拉、下拉、复用功能归零都好理解；关键是 **ANALOG 位**：
CW32L012 复位后 PC13 处于**模拟态**（`GPIOx_ANALOG.PIN13 = 1`）——
这是实机踩出来的坑。模拟态下数字输出驱动器是断开的，如果 HAL 只写
DIR 不清 ANALOG，寄存器看着都对、LED 就是不闪。`into_output` 顺手
把这一位清掉，例程才能"一行点灯"。反过来，第 11 章的 ADC/OPA 引脚
需要的是模拟态，`Pin::new` 会清掉它，所以模拟用途必须显式调
`into_analog()` 置回去——HAL 文档里记录过 PB00 未置模拟导致运放输出
被数字驱动器拉死的实测案例。

### 段 4：toggle() 与 TOG 寄存器

```rust,ignore
led.toggle();
```

翻转不是"读 ODR → 异或 → 写回"三步，而是往 `TOG` 寄存器（@0x60）写
位掩码，硬件一次完成翻转。原子操作意味着中断
随时进来都不会撕裂这次电平变更——第 6 章的定时器 ISR 里还会再见到它。

### 段 5：Delay 的校准基准

```rust,ignore
let mut delay = Delay::new(&clocks);
delay.delay_ms(500);
```

`Delay` 用 Cortex-M0+ 的 SysTick 做忙等，时钟源固定为 HCLK，而 HCLK
是多少，来自**构造时传入的 `Clocks` 快照**。`Delay::new` 预计算
"每微秒/每毫秒多少个节拍"两个系数，避开 M0+ 没有的硬件除法
（`u64` 除法要走软件例程，debug 构建下一次约上千周期）。注意推论：
**如果你在 `Delay::new` 之后改了主频，延时就不准了**——它不会跟着
时钟走。先定频，再建 `Delay`。

## 4.4 板上验证

**状态：同源已验证。** 本章例程与 cw32l012-hal 仓库的
`examples/blink_pc13.rs` 逐行同源（仅按 edition 2024 改写
`#[unsafe(no_mangle)]` 并调整注释）。原例在 cw32l012-hal 的
`TESTING.md`（测试备忘录，2026-10-17 ~ 10-19 实测）"实测通过"表中有记录：

| 例程 | 验证内容 | 关键读数 |
|------|----------|----------|
| blink_pc13 | GPIO KEY 门控→ANALOG 清零→BSRR/TOG | PC13 500ms 方波 |

即：复位默认时钟、AHBEN 门控、ANALOG 清零、TOG 翻转这条完整链路
在实板上以 500ms 方波的形式得到过确认，寄存器回读与手册一致。
延时精度另有 `clk_check` 条目：`delay_ms(1000)` 恰好 1000 节拍，
`delay_us` 误差 +0.5%~+1.1%（4 MHz 档）。

## 4.5 常见坑

- **忘了时钟门控**：直接手写 GPIO 寄存器而不开 AHBEN 位，写入全部
  静默丢弃。HAL 的 `Pin::new` 自动开，自己绕过 HAL 时最容易栽在这里。
- **写门控忘带 KEY=0x5A5A**：同样静默丢弃，没有任何错误标志可查。
- **PC13 复位默认 ANALOG=1**：不清这一位，DIR 配了也不输出——
  任何"寄存器全对但引脚没波形"的怪象先查 ANALOG。
- **DIR 方向语义与 STM32 相反**：CW32 是 0 输出、1 输入（§9.6.1），
  从 STM32 迁移来的代码按旧直觉改 DIR 会得到反效果。
- **先 Delay::new 后改主频**：延时以构造时的 HCLK 快照为准，换频后
  需要重建 `Delay`，否则 500ms 可能不是 500ms。
