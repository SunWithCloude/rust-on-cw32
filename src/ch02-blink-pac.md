# 第 2 章 从零点灯：PAC 与寄存器

这一章你会学到：

- 一个 `no_std` 裸机工程的"四件套"：Cargo.toml、memory.x、build.rs 与链接约定；
- PAC（寄存器访问层）的角色：把"往地址读写"变成"类型化的寄存器对象"；
- L012 的外设时钟门控与 `KEY = 0x5A5A` 写保护；
- PC13 点灯的完整寄存器路径：门控 → 退模拟态 → 设方向 → 位翻转；
- `volatile` 到底防的是什么——编译器会替你"优化"掉哪些东西。

本章不用 HAL、不用任何封装：一行寄存器对应一次总线写。理解了这一章，
后面所有 HAL/异步封装对你都是透明的。

## 裸机工程四件套

第 1 章你已经编译过 `examples/pac` 这个 crate，现在打开看它的全部家当。
除了 Cargo.toml，一个能跑在 Cortex-M0+ 上的裸机工程还需要三样东西。

**其一，依赖声明。** `examples/pac/Cargo.toml` 的关键部分：

```toml
[package]
name = "cw32-book-pac"
edition = "2024"
publish = false

[dependencies]
cw32l012-pac = { path = "../../../cw32l012-pac", features = ["rt"] }
```

`features = ["rt"]` 让 PAC 额外生成两样链接期的东西：`__INTERRUPTS` 中断
向量表和 `device.x` 中断符号表（详见 PAC 的 `docs/rt-smoke.md`）。没有
`rt`，PAC 就只是"一堆地址常量"，链接出来的固件没有向量表，复位后 CPU
取不到第一条指令。

**其二，memory.x。** 告诉链接器这块芯片有多少内存、放在哪：

```text
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 64K
  RAM   : ORIGIN = 0x20000000, LENGTH = 8K
}
```

CW32L012 是 64 KB Flash / 8 KB SRAM，两个数值直接来自数据手册容量规格。
`cortex-m-rt` 的 `link.x` 会读这个文件来安排向量表、`.data`/`.bss` 的位置。

**其三，链接参数的下发方式。** `examples/pac/build.rs` 全文（注释除外）：

```rust
use std::env;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=memory.x");
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rustc-link-search=native={}", manifest.display());
    println!("cargo::rustc-link-arg=-Tlink.x");
}
```

它把 `memory.x` 所在目录加进链接搜索路径，并给所有二进制附加
`-Tlink.x`（cortex-m-rt 的链接脚本）。为什么不用 `.cargo/config.toml`
写 `rustflags`？因为 `.cargo/config.toml` 只在"以本 crate 目录为当前
目录"构建时才被 cargo 读取——而本书的验收命令（第 1 章）是从工作区根
执行的，那时子目录里的 config 根本不会被看。`build.rs` 下发的链接参数
则始终生效。

`.cargo/config.toml` 文件本身保留（只有几行注释，说明链接脚本由
build.rs 负责），将来放 runner 之类本目录专属配置。

## 外设是地址，PAC 是类型

芯片手册第一章的存储器映射表（表 2-1）告诉你：SYSCTRL 外设的寄存器组
从地址 `0x4000_4000` 开始，GPIOC 的从 `0x4800_0200` 开始。裸机编程的
本质就是往这些地址读写。

PAC 做的事，是把这些地址变成**有类型的对象**：

- 每个外设一个 `struct`（如 `Sysctrl`、`Gpio`），字段按寄存器偏移排布，
  `repr(C)` 保证内存布局与手册逐字对齐；
- 每个寄存器一个访问类型：`Rw<u32>` 可读可写、`Ro<u32>` 只读、
  `Wo<u32>` 只写，分别只暴露合法的方法（`read`/`write`/`modify`）；
- 每个寄存器的位段和取值是常量（如 `sysctrl::AHBEN_GPIOC`），API 名
  直接从手册转录。

入口是外设单例：

```rust
let p = Peripherals::take().unwrap();
```

`Peripherals::take()` 只能成功一次，第二次调用返回 `None`——用 Rust 的
所有权机制保证"一个外设不会被两段代码同时随意配置"。`p.sysctrl`、
`p.gpioc` 这些字段背后就是上面那两个地址，`p.gpioc.analog` 是一个
`Rw<u32>`，对它调用 `read()`/`modify()` 展开为一条 volatile 访存指令。

> 一条纪律：**固件里必须真正用到 PAC**。哪怕只是 `Peripherals::take()`
> 这一行，也能让 PAC 的 rlib 进入链接命令行；如果整个程序一个 PAC 符号
> 都不引用，链接器会报 `The interrupt vectors are missing`——不是向量表
> 写错了，而是它压根没被拉进来（PAC `docs/rt-smoke.md` 记录的这个坑，
> 本章例程不会踩到，因为 `take()` 本身就引用了 PAC）。

## 第 1 步：时钟门控与 KEY 写保护

CW32L012 复位后，**除 SysTick 和 SRAM 外的所有外设时钟都处于关闭状态**
（手册 §2）。手册明确写了不开配置时钟的后果：写入数据失败、**且不会产生
HardFault**，读出的数据不可用——错误静默发生，这是这类芯片最容易踩的坑。
所以点灯第 1 步永远是打开 GPIOC 的时钟。

GPIO 时钟走 AHB 门控寄存器 `SYSCTRL_AHBEN`（手册 §4.7.11，偏移 0x30，
表 4-9）：bit4/5/6/7 分别是 GPIOA/B/C/F。要点亮 PC13，置 bit6：

```rust
p.sysctrl.ahben.modify(|v| v | sc::AHBEN_GPIOC);
```

——**这样写是无效的**。`AHBEN` 带 KEY 写保护：高 16 位是 KEY 字段，
只写，**仅当写入值的高 16 位为 `0x5A5A` 时这次写操作才有效**（手册
§4.7.11 位表原文："仅当 KEY 为 0x5A5A 时，对该寄存器的写操作有效"；
§4.5 的流程说明同样强调"写入的数据高 16bit 数据必须是 0x5A5A，否则无法
写入"）。PAC 把这个键值做成了常量：

```rust
/// 写保护键：…高 16 位必须为 0x5A5A，否则写入无效
pub const KEY: u32 = 0x5A5A << 16;
```

（PAC `src/lib.rs` 的 `sysctrl` 模块。）

麻烦在于 KEY 是只写字段，**读出来恒为 0**（HAL `rcc.rs` 的注释同样
记录了这一点）。于是"改一个位"的标准序列变成三段：先读出寄存器、把
高 16 位 KEY 清掉（反正读出来是 0，用 `& 0xFFFF` 表达意图），再把
KEY 原样补回、或上要置位的位，一次写回。这正是 `cw32l012-hal`
`rcc.rs` 中 `set_bits` 的实现，本章例程逐字采用同一步序：

```rust
let v = p.sysctrl.ahben.read() & 0xFFFF;        // 低 16 位是门控状态
p.sysctrl
    .ahben
    .write(sc::KEY | v | sc::AHBEN_GPIOC);      // KEY + 现状 + GPIOC(bit6)
```

注意这不是 `modify()`——`modify` 是"读、闭包加工、写回"，如果闭包
不知道要补 KEY，写回的值高 16 位是 0，整次写入被硬件丢弃。KEY 保护
寄存器（CR0/CR1/CR2/AHBEN/APBEN1/2/复位寄存器等）一律要走"补 KEY"
这条路。

## 第 2 步：把 PC13 从模拟态领回来

L012 每个引脚复位后默认处于**模拟态**：`GPIOx_ANALOG` 的对应位为 1。
手册 §9.6.7 给出复位值——GPIOC 是 `0x0000_E000`，即 PC13/14/15 三个
引脚全部默认模拟态。模拟态下"端口的数字功能关闭"（手册 §9.3）：数字
输入输出被切断。不清这一位，后面写 DIR、写 TOG 全是徒劳。

GPIOC 的寄存器组在 PAC 里就是一个 `Gpio` 结构体，`analog` 字段偏移
0x1C。清掉 PIN13 位：

```rust
p.gpioc.analog.modify(|v| v & !PIN13);
```

普通可读可写寄存器，这里用 `modify()` 正合适（读、清位、写回）。

## 第 3 步：方向设为输出

方向寄存器 `GPIOx_DIR`（手册 §9.6.1）有一个和多数 MCU（包括 STM32）
**相反**的约定：

- `DIR.PINy = 1` 是**输入**；
- `DIR.PINy = 0` 是**输出**。

PAC 把这对取值做成常量 `gpio::DIR_INPUT = 1` / `gpio::DIR_OUTPUT = 0`。
GPIOC 的 DIR 复位值同样是 `0x0000_E000`（§9.6.1）——PC13 复位默认
输入态。要输出就得清位：

```rust
p.gpioc.dir.modify(|v| v & !PIN13);
```

输出模式（推挽/开漏）由 `GPIOx_OPENDRAIN` 控制，复位默认推挽，本例
不用动。

## 第 4 步：TOG 翻转 + 软件延时

点亮和熄灭，最省事的是位翻转寄存器 `GPIOx_TOG`（手册 §9.6.17）：
**向某位写 1，对应 `ODR` 输出位被硬件原子翻转**（手册 §9.3）。它是
只写寄存器（PAC 里是 `Wo<u32>`），没有读回一说的干扰，也没有
读-改-写三步的竞态：

```rust
p.gpioc.tog.write(PIN13);   // PC13 电平翻转一次
```

`PIN13` 就是 `1 << 13`——GPIO 寄存器的位段一律按引脚号 PINy 编排。

翻转之间需要点时间差，插一个软件延时。**延时的写法有个陷阱**：直接写
`for _ in 0..n {}`，release 构建下 LLVM 看出循环没有任何副作用，会把它
整个删掉——固件"延时"零拍，LED 看起来不闪或狂闪。防御办法是让循环
体对编译器不可见：要么 volatile 访问（编译器不许删 volatile 操作），
要么 `core::hint::black_box`。本例程用 volatile 计数，正好和寄存器访问
是同一原语：

```rust
fn delay(n: u32) {
    let mut left = n;
    unsafe {
        let p = core::ptr::addr_of_mut!(left);
        while p.read_volatile() > 0 {
            p.write_volatile(p.read_volatile() - 1);
        }
    }
}
```

`delay(500_000)` 在复位默认的 HCLK = 4 MHz（HSI，§4.3.4）下大约
0.5~1 秒量级——没有精确标定，也不需要：LED 闪烁只要求"肉眼可见的
节奏"，第 4 章的 HAL 会给出校准过的毫秒延时。

## 例程全文

`examples/pac/examples/ch02_blink.rs`：

```rust
//! 第 2 章例程：PAC 裸寄存器点灯——GPIOC 门控（KEY 写保护）→ PC13
//! 模拟态转输出 → TOG 翻转 + 软件延时。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch02_blink -p cw32-book-pac
//! 状态：仅编译未上板；寄存器路径与 cw32l012-hal 实测例 blink_pc13 同源
//! （TESTING.md：KEY 门控→ANALOG 清零→TOG，PC13 500ms 方波）。

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use cw32l012_pac::{Peripherals, sysctrl as sc};

/// PC13 在端口寄存器中的位掩码（GPIO 位段均按 PINy 编号：PIN13 = bit13）
const PIN13: u32 = 1 << 13;

/// 粗略的软件延时（复位默认 HCLK = 4 MHz 时约 0.5~1 秒量级，未精确标定）。
/// 用 volatile 计数是防止空转循环被优化掉——不带 volatile，
/// release 构建会直接删掉这个循环。
fn delay(n: u32) {
    let mut left = n;
    // SAFETY: 只在本函数内经裸指针 volatile 访问栈上局部变量，单核无竞争
    unsafe {
        let p = core::ptr::addr_of_mut!(left);
        while p.read_volatile() > 0 {
            p.write_volatile(p.read_volatile() - 1);
        }
    }
}

#[entry]
fn main() -> ! {
    // 外设单例：Peripherals::take 只能成功一次（返回 None 即已被取走）
    let p = Peripherals::take().unwrap();

    // ---- 第 1 步：打开 GPIOC 的时钟（SYSCTRL_AHBEN，手册 §4.7.11）----
    // KEY 写保护：AHBEN 高 16 位必须写 0x5A5A，否则整个写入无效（PAC
    // sysctrl::KEY = 0x5A5A << 16）；KEY 位读出恒为 0，所以改位前先读
    // 低 16 位现状、写入时把 KEY 原样补回（与 cw32l012-hal rcc::set_bits
    // 同一步序）。
    let v = p.sysctrl.ahben.read() & 0xFFFF;
    p.sysctrl
        .ahben
        .write(sc::KEY | v | sc::AHBEN_GPIOC); // AHBEN.GPIOC = bit6（§4.7.11）

    // ---- 第 2 步：PC13 从模拟态切回数字（GPIOC_ANALOG，手册 §9.6.7）----
    // 复位默认 GPIOx_ANALOG = 0xE000（GPIOC），PIN13 = 1（模拟功能），
    // 数字输入输出被断开，点灯前必须清零（§9.4.1 步骤 1）。
    p.gpioc.analog.modify(|v| v & !PIN13);

    // ---- 第 3 步：方向设为输出（GPIOC_DIR，手册 §9.6.1）----
    // 注意与多数 MCU 相反：DIR.PINy = 1 是输入，0 是输出（PAC gpio::
    // DIR_OUTPUT = 0）；GPIOC 复位值 0xE000，PC13 默认输入，这里清位
    // （§9.4.1 步骤 2）。
    p.gpioc.dir.modify(|v| v & !PIN13);

    loop {
        // ---- 第 4 步：翻转输出（GPIOC_TOG，手册 §9.6.17）----
        // TOG 是只写寄存器：写 1 的位，对应 ODR 位被硬件原子翻转
        // （§9.3），不用读-改-写。
        p.gpioc.tog.write(PIN13);
        delay(500_000);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
```

`#![no_std]` 排除标准库，`#![no_main]` 交出 `main` 的定义权，
`#[entry]`（cortex-m-rt）把函数标记为复位入口，并要求签名 `fn() -> !`
（入口永不返回）。panic 处理器是能满足编译器要求的最小形式——本章
还没有日志通道，panic 了只能原地空转（第 3 章起 panic 会说话）。

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch02_blink -p cw32-book-pac
```

产物在 `target/thumbv6m-none-eabi/release/examples/ch02_blink`。烧录到
板上运行（`probe-rs download` + `probe-rs reset`，命令模板见附录 D），
PC13 板载 LED 应以约 1 秒为周期明暗交替。

## 板上验证

**本例程本身：仅编译未上板**（thumbv6m-none-eabi release 构建通过，
`__INTERRUPTS` 向量表正常落位）。

但例程的寄存器写序列与已实机验证的路径**同源**：cw32l012-hal 的
`blink_pc13` 例程在同一块板上验证通过——`TESTING.md`（2026-10-17）记录
其验证内容为"GPIO KEY 门控→ANALOG 清零→BSRR/TOG"，关键读数
"PC13 500ms 方波"。本例程与它的差异仅在：不走 HAL 封装、延时未精确
标定（节奏约 1 s 而非 500 ms）。

首次上板时值得核对的三件事：

1. LED 不闪时，优先怀疑门控/ANALOG 两步漏写（错误静默，见 §2 的
   "不会产生 HardFault"）；
2. 亮度正常但不闪，说明 TOG 在跑而延时被优化——检查构建是否 release
   且延时用了 volatile；
3. 用 probe-rs 断点停在 `main`，单步观察 `ahben` 写入前后
   `p.sysctrl.ahben.read()` 低 16 位的变化。

## 常见坑

- **忘开 GPIOC 时钟**：所有后续寄存器写静默失败（无 HardFault），LED
  纹丝不动。任何外设上手第一步都是查 `SYSCTRL_AHBEN`/`APBEN`。
- **KEY 忘补**：`modify(|v| v | sc::AHBEN_GPIOC)` 对 KEY 保护寄存器
  无效——读出的高 16 位是 0，写回的整值被丢弃。KEY 寄存器必须
  "读低 16 位 + 写回时补 `sc::KEY`"。
- **DIR 方向记反**：L012 的 `DIR=1` 是输入。方向不对时引脚既不输出
  也不报错，读到的是输入缓冲的内容。
- **ANALOG 忘清**：复位默认 PC13 是模拟态，数字输出被切断；症状与
  忘开时钟一样是"静默无效"。
- **延时循环被优化掉**：release 下空转循环会整个消失，用 volatile 或
  `black_box` 保住它（第 4 章起改用硬件定时器延时）。
- **删掉了 `Peripherals::take()`**：若固件不再引用任何 PAC 符号，链接
  报 `The interrupt vectors are missing`。
