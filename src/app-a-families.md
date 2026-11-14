# 附录 A 11 家族矩阵与移植

embassy-cw32 与 cw32-bsp 覆盖武汉芯源 CW32 全部 **11 个 Cortex-M0+ 家族**。
本书正文以 CW32L012 为教学主线；这一附录回答两个问题：

1. 其他家族的现状如何（PAC 从哪来、驱动覆盖到哪、硬件差异是什么）；
2. 把本书 L012 例程搬到另一块板子上，要改哪几处。

> 表中每个格子都从生态仓 README/CHANGELOG/源码头注释核对而来，出处随格标注。
> 拿不准的地方明确写"未核实"，不编数据。

## A.1 家族矩阵

**PAC 来源**分两类：CW32L012 的 `cw32l012-pac-um` 是**手写**——逐项转录自
《CW32L012 用户手册》V1.4，与官方 SVD 逐项纠错（出处：cw32l012-pac README）；
其余 10 个家族的 PAC 由 `cw32-pac-gen` 从**原厂 SDK 头文件生成**，一个家族
一个 crate（出处：cw32-pacs README）。全部 11 家族 PAC 已发布 crates.io
（2026-11-14）：8 家族用原名，L012/F030/L010 三家族因原名被第三方占用，
以 `-um` 后缀（User Manual，手写/生成转录版）发布——下表"PAC crate"列
一律写 crates.io 正式名（命名故事见附录 B）。

| 家族 | PAC crate | Flash/SRAM | embassy feature 名 | 复位默认时钟 | 门控 KEY（0x5A5A） | DMA 通道 | I2C | PAC 收编 GPIO 端口 |
|------|-----------|-----------|--------------------|--------------|--------------------|----------|-----|--------------------|
| CW32L010 | `cw32l010-pac-um`（生成；原名被占） | 64K/4K | `cw32l010` | **4 MHz** | 有 | 无 | 有 | A, B |
| CW32L011 | `cw32l011-pac`（生成） | 64K/6K | `cw32l011` | **4 MHz** | 有 | 无 | **无硬件** | A, B, C |
| CW32L012 | `cw32l012-pac-um`（**手写**，UM V1.4 转录；原名被占） | 64K/8K | `cw32l012`（默认） | **4 MHz** | 有 | **4** | 有（命令 FIFO 控制器） | A, B, C, F |
| CW32L031 | `cw32l031-pac`（生成） | 64K/8K | `cw32l031` | 8 MHz | 无 | 4（L031 组） | 有（经典状态机） | A, B, C, F |
| CW32L052 | `cw32l052-pac`（生成） | 64K/8K | `cw32l052` | 8 MHz | 无 | 4（L031 组） | 有（经典状态机） | A, B, C, D, F |
| CW32L083 | `cw32l083-pac`（生成） | 256K/24K | `cw32l083` | 8 MHz | 无 | **5** | 有（经典状态机） | A, B, C, D, E, F |
| CW32R031 | `cw32r031-pac`（生成） | 64K/8K | `cw32r031` | 8 MHz | 无 | 4（L031 组） | 有（经典状态机） | A, B, C, F |
| CW32W031 | `cw32w031-pac`（生成） | 64K/8K | `cw32w031` | 8 MHz | 无 | 4（L031 组） | 有（经典状态机） | A, B, C, F |
| CW32F003 | `cw32f003-pac`（生成） | 20K/3K | `cw32f003` | 8 MHz | 无 | 无 | 有（中断名 `I2C` 无数字） | A, B, C |
| CW32F020 | `cw32f020-pac`（生成） | 32K/8K | `cw32f020` | 8 MHz | 无 | 5（ch4/5 无 NVIC 线，仅轮询） | 有（经典状态机） | A, B, C, F |
| CW32F030 | `cw32f030-pac-um`（生成；原名被占） | 64K/8K | `cw32f030` | 8 MHz | 无（见 A.2 脚注） | **5** | 有（经典状态机，**实板验证**） | A, B, C, F |

各列出处：

- **Flash/SRAM**：cw32-pacs README 家族矩阵（L012 行来自 cw32l012-pac README）。
  注意这是 PAC `memory.x` 的**家族最大配置**（出处：cw32-pacs README），具体
  型号可能更小——如 CW32F030F6 是 32K（出处：cw32-probe-rs-compat README 变体表）。
- **复位默认时钟**：cw32-bsp README 家族矩阵（L010/L011/L012 = 4 MHz，
  其余 = 8 MHz）；embassy-cw32 CHANGELOG 记录了这组结论的来历——"L031 Rev1.8
  p14 等原文纠正了此前按 4MHz 的归类"，即以各家族数据手册原文为准。
- **门控 KEY**：embassy-cw32 README——L012 代（`cw32l010`/`cw32l011`/`cw32l012`）
  的 SYSCTRL 写受 0x5A5A 键保护，经典代 8 家族无键（普通读改写）
  （出处：embassy-cw32 README；CHANGELOG"门控写规范"条）。
- **DMA**：embassy-cw32 README `dma` 模块行与 CHANGELOG `ClassicDma` 条——
  L012 4 通道（IRQ9 DMA12/IRQ10 DMA34）；经典代 F030/F020/L083 = 5 通道、
  L031 组（L031/L052/R031/W031）= 4 通道；**F003/L010/L011 无 DMA 硬件**。
- **I2C**：embassy-cw32 README `i2c` 模块行——11 家族中 10 个有 I2C，**L011
  无 I2C 硬件**；L012 是命令 FIFO 式控制器，其余 9 家用经典 8051 式状态机控制器。
- **GPIO 端口**：cw32-bsp 各家族 `pins.rs` 表头"本家族 PAC 收编端口"
  （cw32-bsp 6a6b773/4c4218b 再生成）。这是 **PAC Peripherals 实际建模的端口**，
  与各家族芯片的物理端口一致（生成源是厂商 SDK 头文件）。

**关于 F030 的 KEY 脚注**：F030 的 AHBEN/APBEN 门控寄存器**没有**键保护，
但 CR0/CR1/IER 等外设寄存器仍带键——`rcc` 模块的 PLL 配方（转录自厂商库）
负责这些带键写（出处：embassy-cw32 README "F030 family notes"）。

**未核实的格子**：无。11 家族 × 8 列全部有仓库出处。但"实机验证"覆盖差异很
大：只有 L012 与 F030 有实板记录（见 A.3）。注意一个口径差异——cw32-pacs
README 的"实机验证"列对**全部 10 个家族**（含 F030）都写"寄存器层待上板"，
而 embassy-cw32 仓已有 F030 实板记录（LCKFB CW32F030C8T7，见下）——前者是
未及时回收的滞后口径，以 embassy-cw32 的实机记录为准（出处：cw32-pacs
README 实机验证列；embassy-cw32 README "F030 family notes"）。

## A.2 embassy-cw32 模块覆盖面

移植前先确认目标家族有没有你要用的模块（出处：embassy-cw32 README
"What you get" 表）：

| 模块 | 覆盖家族 | 说明 |
|------|----------|------|
| `time_driver` / `bind_interrupts!`（critical-section 实现由 cortex-m 的 `critical-section-single-core` 随依赖带上） | 全部 11 家族 | SysTick 单次闹钟时间驱动（1 MHz tick）；中断绑错编译期报错 |
| `gpio`（异步） | 全部 11 家族 | 一端口一共享 IRQ；l052/l083 的共享向量 `GPIOC_GPIOD`/`GPIOE_GPIOF` 成对派发 |
| `uart` | 全部 11 家族 | L012/F030 有 UART1/2/3，**其余家族只有 UART1**；`BufferedUart` 全家族可用 |
| `spi` | 全部 11 家族 | L012 SPI1/2/3，F030 SPI1/2，其余单实例 |
| `i2c` | 10/11（L011 除外） | L012 命令 FIFO 式；其余 9 家经典状态机式（F030 已实板验证） |
| `adc` | L012 + F030/F020（0.4.0 起补齐全部 11 家族） | L012 双 ADC（含 `vdda_mv()`/`temperature_mc()`）；经典组是完全不同的 IP，单发/4 结果序列 |
| `timer`（PWM） | **仅 L012**（0.4.0 起经典 GTIM PWM 落地） | GTIM1-4 边沿对齐 PWM，万分比满量程；经典代无影子寄存器、立即生效 |
| `rtc` / `wdt` / `dac` | **仅 L012**（0.4.0 起 rtc/wdt 补齐经典代） | 经典代为 `ClassicRtc`/`ClassicIwdt`/`ClassicWwdgt`（另一套 IP）；`dac` 仍仅 L012 |
| `dma` | L012（`Dma`）+ 经典 7 家族（`ClassicDma`） | F003/L010/L011 无 DMA 硬件，自然无驱动 |

> **0.4.0 增量一句话版**（出处：embassy-cw32 CHANGELOG 未发布段）：经典家族
> UART/SPI DMA、ADC 补齐全部 11 家族、经典 GTIM PWM、经典 RTC/IWDT/WWDGT、
> L012 FLASH 驱动、L012 OPA、I2C transaction——这些是新驱动，本书正文暂不
> 展开，移植前先读该 CHANGELOG。

实机验证记录（哪块板验证了什么）：

- **CW32L012**：blink_async（2026-10-24，PC13 精确 1 秒周期、SysTick 闹钟
  实测 1,997,499 ≈ 2,000,000 systicks）、buffered_echo（含注入 UART1 pend
  走通向量→`bind_interrupts!`→critical-section 全链路）；uart_echo_async
  已烧录运行无硬件故障，回环测量尚待 USB-串口适配器
  （出处：embassy-cw32 README "Verification status"；时间驱动基数 bug 的
  上板排障记录也在其中）。
- **CW32F030**（立创天猛星 CW32F030C8T7）：blink_async_f030（500 ms 心跳）、
  UART1 初始化链（PA08/PA09 AF2，BRRI=4 @115200）、SPI1（CR1=0x1E7C）、
  i2c_scan_f030（112/112 地址 NACK）（出处：embassy-cw32 README "F030
  family notes"、CHANGELOG I2C 多家族化条）。

## A.3 把 L012 例程移植到其他家族

以把第 7 章的异步 blink 搬到 **CW32F030** 为例（搬去其他经典家族同理，
L012 代家族更简单——见下）。

### 第 1 步：换 feature 名与依赖

embassy-cw32 靠互斥 feature 选家族（出处：embassy-cw32 README）：

```toml
[dependencies]
embassy-cw32 = { git = "https://github.com/SunWithCloude/Cw32l012_embassy", branch = "main", features = ["rt"] }
# F030：关掉默认 feature（默认是 cw32l012），改开 cw32f030
# embassy-cw32 = { git = "https://github.com/SunWithCloude/Cw32l012_embassy",
#   branch = "main", default-features = false,
#   features = ["rt", "cw32f030"] }
# （embassy-cw32 发布 crates.io 后改回版本依赖即可）
```

命令行等价形式（出处：embassy-cw32 README "F030 family notes"）：

```bash
cargo build --target thumbv6m-none-eabi --release \
  --no-default-features --features "rt,cw32f030"
```

PAC 随 feature 自动切换，全部走 crates.io 版本依赖：L012 用
`cw32l012-pac-um`（`=0.3.0`）、F030/L010 用 `cw32f030-pac-um`/
`cw32l010-pac-um`，其余 8 家用原名（如 `cw32f003-pac`、`cw32l031-pac`，
全部 `=0.1.2` 精确钉版）——不需要克隆任何 PAC 仓（出处：embassy-cw32
README "Getting started" 与 Cargo.toml）。

阻塞侧没有得选：`cw32l012-hal` **只覆盖 L012**，本书第二部分例程无法直接
"换 feature" 移植到其他家族——阻塞 HAL 目前就是 L012 专属
（出处：cw32l012-hal README；cw32-bsp README 生态图）。

### 第 2 步：核对复位默认时钟

- L012 代（L010/L011/L012）= 4 MHz，经典代 8 家族 = 8 MHz
  （出处：cw32-bsp README；embassy-cw32 CHANGELOG）。
- `Config::default()` 已经按家族声明了复位默认频率（4 MHz HSI 96M/24 与
  8 MHz HSI 48M/6），不改时钟树就不用动
  （出处：embassy-cw32 README "Clock convention"）。
- **反例警示**：曾假设 F030 复位后跑 48 MHz，结果 SysTick 慢 6 倍、blink
  周期变成 6 秒——上板排障发现，DFP 的 `Dclock=8000000` 佐证了 8 MHz
  （出处：embassy-cw32 CHANGELOG 修复条）。这正好是 CONVENTIONS §5
  "时钟倍频错"坑的家族版。

### 第 3 步：门控写法

- L012 代：AHBEN/APBEN 是**带键写**（KEY = 0x5A5A），第 2 章的
  `sysctrl::KEY | sysctrl::AHBEN_GPIOA` 就是这种（出处：cw32l012-pac README
  用法示例）。
- 经典代：AHBEN/APBEN **无键**，用普通读改写（`modify`）
  （出处：embassy-cw32 CHANGELOG "门控写规范"条，与 cw32-hal2 各家族
  clock sidecar 的 `enable_write_key` 元数据交叉验证）。
- 走 embassy 驱动（如 `sysctrl_enable_gpioc`）则不需要操心——封装已按家族
  区分（出处：embassy-cw32 README 示例）。

### 第 4 步：查引脚复用（pins 模块）

引脚/AF 组合因家族而异，**不要照抄 L012 的引脚号**。cw32-bsp 每个家族的
`pins` 模块给出 UART1/SPI1/I2C1 的全部可用 `(端口, 引脚号, AF 编号)` 组合，
逐条带厂商 SDK 宏出处（数据源为官方数据手册 AF 表的定位提取）
（出处：cw32-bsp README）：

- L012：UART1 = PA02(TX)/PA03(RX) **AF1**（实机验证，CONVENTIONS §5）；
- F030：UART1 = PA08(TX)/PA09(RX) **AF2**（实机验证，出处：cw32-bsp README
  实板支持列）。

查法（以 F030 为例）：

```rust
use cw32f030_bsp::pins;
// pins::UART1_TXD / pins::UART1_RXD 列出全部组合
let tx = bsp_pin!(A, p, 8); // PA8 = UART1_TX
let rx = bsp_pin!(A, p, 9); // PA9 = UART1_RX
```

注意 F030 复位默认 GPIO 处于 ANALOG 态（`0x9fff`，仅 SWD 两根线是数字使能），
DIR 保持输入态时 UART 的 START 永远发不出去——embassy 驱动的 `I2c::new_inner`
等已统一处理，裸寄存器移植时要自己清
（出处：embassy-cw32 CHANGELOG I2C 多家族化条）。这与 L012 的"PC13 复位默认
模拟态"（CONVENTIONS §5）是同一类坑，家族间的差别在默认值。

### 第 5 步：单端口/少端口家族的 blink

部分家族的 `bsp_pin!` 宏当前只展开 A 口（见 A.5 同步注记），移植 blink 时
先落 **PA0** 起步，再按原理图改到 LED 实际所在引脚：

```rust
let mut led = Output::new(Pin::new(p.gpioa, 0), Level::High); // PA0
```

经典家族 LED 常在 PC13（F030 实板就是 PC13——出处：cw32-bsp README 实板
支持列），此时 PAC 已收编 C 口，`Pin::new(p.gpioc, 13)` 可用。

### 第 6 步：中断名与实例数

- `bind_interrupts!` 里的 IRQ 名要换**所选家族 PAC 的 `Interrupt` 枚举**
  变体（两套 PAC 的枚举命名不同——出处：embassy-cw32 CHANGELOG
  `nvic_enable` 条）。例如 l083 的 GPIO 中断是共享向量 `GPIOC_GPIOD`、
  F003 的 I2C 中断名无数字、就叫 `I2C`（出处：embassy-cw32 CHANGELOG）。
- UART/SPI 实例数按 A.2 表核对：F030 有 UART1/2/3，L031 只有 UART1——
  `Uart::new_uart2` 在单 UART 家族上不存在，编译器会直接告诉你。

### 第 7 步：L012 独占模块的去留

`timer`（PWM）、`rtc`、`wdt`、`dac` 的 L012 版本书里逐章展开；0.4.0 起
embassy-cw32 为经典代补齐了 GTIM PWM、RTC、IWDT/WWDGT，并给全部 11 家族
配了 ADC 驱动（出处：embassy-cw32 CHANGELOG 未发布段），本书正文尚未
覆盖这些新驱动。移植第 10/11/13 章到非 L012 家族时，要么改走寄存器层
（对应家族 PAC），要么直接改用新驱动（以该 CHANGELOG/README 为准）。

### 第 8 步：烧录与调试参数

芯片名与 target 描述文件按家族换——L010/L011/L012 在 probe-rs ≥ 0.31.0
内置（`--chip CW32L012x8` 等），其余家族要用 cw32-probe-rs-compat 仓的
target yaml + `--chip-description-path`。详见附录 D
（出处：cw32-probe-rs-compat README）。

两个 Flash 细节坑（出处：cw32-probe-rs-compat README "已知限制"）：
W031 的 Flash 页是 **128 B**（其余系列 512 B）；F003 的 Flash 是 **20 KB**
（`0x5000`，非 2 的幂）——写 flash 自编程移植时留意。

## A.4 移植速查卡

| 要改什么 | L012 代（L010/L011） | 经典代（其余 8 家族） |
|----------|----------------------|------------------------|
| embassy feature | `cw32l010` / `cw32l011` | 对应家族名（如 `cw32f030`） |
| 复位时钟假设 | 4 MHz | 8 MHz |
| 门控写 | KEY 0x5A5A 带键写 | 无键读改写 |
| UART 错误位 | CR1 bit 8..11 | FE=bit3/PE=bit4 |
| blink 引脚 | PA0 起步 | PC13（F030 实板）/ PA0 |
| probe-rs | 内置（≥ 0.31.0） | 本地 target yaml |
| DMA | 无（L010/L011） | `ClassicDma`（F003 除外） |
| I2C | L010 有 / L011 无 | 有（经典状态机） |

（UART 错误位出处：embassy-cw32 CHANGELOG——L012 代"UART 错误位 8..11"，
经典代"错误位 3/4"。其余行出处同前文各节。）

## A.5 两点数据同步注记

移植时以下两处以仓库最新为准（本书写作时的口径）：

1. **`bsp_pin!` 宏与 `pins.rs` 表头存在不同步**：cw32-bsp 6a6b773/4c4218b
   两个提交重新生成了各家族 `pins.rs`（PAC 回填端口后解锁 gpiob+），但部分
   家族 `lib.rs` 里的 `bsp_pin!` 宏仍只展开旧端口集合（如 l083 的表头写
   "收编端口:PA..PF"，宏只展开 A）。落地非 A 口引脚前，先看目标家族
   `pins.rs` 表头，宏不认的端口按其 `[PAC]` 标注处理
   （出处：cw32-bsp `crates/<family>-bsp/src/lib.rs` 与 `src/pins.rs` 对照）。
2. **经典组 DMA 通道数**：embassy-cw32 CHANGELOG 早段 dma 条写"6 通道"，
   后来落地的 `ClassicDma` 条与 README 均写 F030/F020/L083 = 5、L031 组 = 4。
   以 README/`ClassicDma` 条为准（5/4），疑为早期调研口径未回收。
