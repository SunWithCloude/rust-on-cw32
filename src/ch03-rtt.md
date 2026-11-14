# 第 3 章 让芯片开口说话：RTT 日志

这一章你会学到：

- RTT（Real-Time Transfer）是什么，为什么打日志不用 UART；
- defmt 日志框架：格式串进 ELF、主机端解码的工作方式；
- `defmt` + `defmt-rtt` + `panic-probe` 三件套，以及裸机 M0+ 上必须自己
  提供的 critical-section 实现；
- `_defmt_timestamp` 的作用，defmt 1.x 用 `timestamp!` 宏提供时间戳；
- 用 `probe-rs run` 一条命令完成烧录、运行、收日志。

第 2 章的 LED 只会说"闪/不闪"，出了 bug 你只能瞪着它。从这一章起，
芯片能主动向电脑汇报——不占用任何 UART 引脚，一根调试线全搞定。

## RTT 是什么

RTT（Real-Time Transfer）是 SEGGER 设计的调试通道协议，思路很直接：
固件和调试器约定一块**位于 SRAM 的环形缓冲区**（控制块 `_SEGGER_RTT`），
固件往里写，调试器经 SWD 把它读走。CPU 侧只是几次内存写，微秒级完成，
不阻塞程序——这就是"实时"的含义。

对比 UART 打日志的老路，RTT 的优势在本板上非常具体：

- **不占引脚**。UART1 占 PA02/PA03；RTT 走调试线，四根 SWD 线之外一根
  都不多接。
- **不用对波特率**。UART 两端要约定同样的波特率，差 2% 就出乱码；RTT
  是内存读取，没有速率协商。
- **速度快**。SWD 时钟几 MHz 起，远高于 UART 的 115200。

代价：RTT 需要"活的"调试器在线读取——拔掉调试器日志就断了。调试期
日志用 RTT，量产期对外通信才用 UART，两者不冲突。

## defmt：日志也要省空间

RTT 解决"怎么把字节送出来"，[defmt](https://defmt.ferrous-systems.com/)
解决"送什么字节"。它把 `info!("heartbeat #{}", n)` 这类日志编译成紧凑
的二进制帧：**格式串不进固件**，而是 intern 到 ELF 的 `.defmt` 元数据
段里，固件里只带一个 2 字节索引；数值参数按紧凑编码写入帧。主机端
（probe-rs）读 ELF 里的元数据表，把帧还原成人话。

对 64 KB Flash 的芯片，这个设计不是奢侈而是刚需——每个格式串省下的
几十字节乘以日志条数，很快就够省出一个功能模块。

defmt 的日志宏按级别分层：`trace!`/`debug!`/`info!`/`warn!`/`error!`。
级别过滤在**编译期**完成（环境变量 `DEFMT_LOG` 控制），被滤掉的日志
一行字节都不占。

## 三件套：defmt-rtt、panic-probe、critical-section 实现

`examples/pac/Cargo.toml` 的开发依赖（仅对 ARM 裸机目标启用）：

```toml
[target.'cfg(all(target_arch = "arm", target_os = "none"))'.dev-dependencies]
cortex-m-rt = { version = "0.7", features = ["device"] }
critical-section = { version = "1", default-features = false, features = ["restore-state-u8"] }
defmt = "1"
defmt-rtt = "0.4"
panic-probe = { version = "0.3", features = ["print-defmt"] }
```

三个 crate 各司其职：

- **`defmt-rtt`**：注册 defmt 的全局 logger，把 defmt 帧写进 RTT 上行
  通道。它在 SRAM 里放好 `_SEGGER_RTT` 控制块（1024 字节环形缓冲，
  可用环境变量 `DEFMT_RTT_BUFFER_SIZE` 调整），调试器按控制块符号找到
  它。注意它和 `rtt-target`（普通文本 RTT 库）**不能共存**——控制块
  只能有一个，defmt-rtt 从源码上禁止了混用。
- **`panic-probe`**（`print-defmt` feature）：提供 panic 处理器。panic
  时经 defmt 打印 panic 位置再停机，替代第 2 章那个只会空转的 handler。
- **`critical-section`**：一个"谁来提供临界区"的约定。`defmt-rtt` 的
  日志锁用 `critical_section::acquire()` 实现，而**裸机 M0+ 没有现成的
  实现**——谁都不提供时，链接直接报：

  ```text
  rust-lld: error: undefined symbol: _critical_section_1_0_acquire
  ```

  解法是固件自己 `set_impl!` 一个：单核芯片的标准做法就是"读 PRIMASK
  → `cpsid i` 关中断"，退出时仅当初"中断原本开着"才 `cpsie i` 恢复。
  例程里的实现与 cortex-m 的 `critical-section-single-core`（embassy
  侧的实现提供者）、`cw32l012-hal` 内置的版本同一写法。**一个固件只能
  有一个实现**（同时启用两个会重复定义）——本书靠三个例程 crate 彼此
  隔离来保证这一点。

restore-state 选 `u8` 不是随手写的：cargo 编译整个工作区时会把各 crate
的 critical-section feature 合并统一，`restore-state-*` 各档**互斥**，
一次编译里出现两档直接编译报错——整张依赖图只能有一个取法。embassy-cw32
当前的依赖链统一在 `u8`（它 0.4.0 起的实现将由 cortex-m 的
`critical-section-single-core` 提供，届时全书例程的取法随之对齐）。

## `_defmt_timestamp`：每条日志的序号

defmt 允许每条日志携带一个"时间戳"值。设备端需要提供一个符号名为
`_defmt_timestamp` 的函数，每打一条日志，defmt 就调用它、把返回内容
编码进帧；主机端解码时把这个值显示在每行开头。

defmt **0.3** 时代的做法是应用手写一个裸符号：

```rust
#[no_mangle]
fn _defmt_timestamp() -> u64 { /* uptime 毫秒 */ }
```

defmt **1.x** 改成了宏——签名也变了（函数接收 `Formatter`，由宏代写）：

```rust
static SEQ: AtomicU32 = AtomicU32::new(0);
defmt::timestamp!("{=u32}", SEQ.load(Ordering::Relaxed));
```

`timestamp!` 全 crate 图只能用一次。不提供它也能链接：defmt 自带的
`defmt.x` 链接脚本里有 `PROVIDE(_defmt_timestamp = __defmt_default_timestamp)`
——兜底是一个空实现，主机端看到 ELF 里只有默认符号，就不显示时间戳
这一列。

本例程没有时钟驱动，时间戳给的其实**不是时间**，而是"这是第几条
日志"的序号——defmt 只负责把数字原样带给主机。想看到真正的毫秒
uptime，需要先有一个时间源：embassy 生态的 `rtt_hello` 用 embassy-time
提供（本书第 7 章进入异步世界后就是这条路线）。

顺带一个 M0+ 的真实限制：`SEQ` 计数不能写
`SEQ.fetch_add(1, …)`——Cortex-M0+ 没有原子"读-改-写"指令，`core` 的
`fetch_add` 在 `thumbv6m-none-eabi` 目标上**不存在**（`load`/`store`
有）。例程里用 load、加一、store 三步走，单核且未开中断时这是安全的。

## 例程全文

`examples/pac/examples/ch03_rtt.rs`：

```rust
//! 第 3 章例程：RTT 日志——defmt 帧经 SEGGER-RTT 上行通道送调试器，
//! `probe-rs run` 原生解码，不占用任何 UART 引脚。
//!
//! 延时与心跳灯改用第 2 章工程化小节诞生的 `board` 封装
//! （`board::delay_ms` / `board::Led`）：日志打点与 LED 翻转同步，
//! 板上首跑时"灯在闪而日志没滚出来"即 RTT 链路问题。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch03_rtt -p cw32-book-pac
//! 状态：仅编译未上板。依赖组合（defmt 1 + defmt-rtt 0.4 +
//! panic-probe(print-defmt)）与 embassy-cw32 的 rtt_hello 完全一致，
//! 后者提交记录的验证级别为 "release link clean"（链接通过）。

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m_rt::entry;
use cw32_book_pac::board::{Led, delay_ms};
use cw32l012_pac_um::Peripherals;
use defmt_rtt as _; // 全局 logger：defmt 帧写入 RTT 上行通道
use panic_probe as _; // panic handler：panic 时经 defmt 报告位置后停机

// defmt 1.x 用 timestamp! 宏定义"时间戳提供者"：宏在顶层展开为一个
// #[export_name = "_defmt_timestamp"] 的函数，defmt-rtt 不提供它，链接
// 就靠这里。defmt 0.3 时代是手写 #[no_mangle] fn _defmt_timestamp() -> u64，
// defmt 1.x 起签名改为接收 Formatter、由本宏代写。
// 本例没有时间驱动，时间戳就用"这是第几条日志"的序号——defmt 只负责把
// 数字原样带给主机。要真正的毫秒 uptime 需要时间源（embassy 版 rtt_hello
// 用 embassy-time 提供，本书第 7 章后即是此路线）。
static SEQ: AtomicU32 = AtomicU32::new(0);
defmt::timestamp!("{=u32}", SEQ.load(Ordering::Relaxed));

// defmt-rtt 用 critical-section 保护日志缓冲，而裸机 M0+ 没有现成的
// critical-section 实现，必须由固件自己提供一个，否则链接报
// `undefined symbol: _critical_section_1_0_acquire`。单核标准做法：
// 读 PRIMASK → `cpsid i` 关中断；释放时仅当初"中断原本开着"才恢复。
// 与 cortex-m 的 critical-section-single-core、cw32l012-hal 各自的实现
// 同一写法——一个固件只能有一个实现，本书靠例程 crate 彼此隔离。
struct SingleCoreCs;
critical_section::set_impl!(SingleCoreCs);

unsafe impl critical_section::Impl for SingleCoreCs {
    unsafe fn acquire() -> critical_section::RawRestoreState {
        let primask: u32;
        unsafe {
            core::arch::asm!("mrs {0}, PRIMASK", "cpsid i", out(reg) primask);
        }
        (primask & 1) as u8 // RawRestoreState = u8（dev-dependencies 里钉死）
    }

    unsafe fn release(token: critical_section::RawRestoreState) {
        if token == 0 {
            unsafe { core::arch::asm!("cpsie i") };
        }
    }
}

#[entry]
fn main() -> ! {
    // 引用 PAC：让向量表随 rlib 进入链接（PAC docs/rt-smoke.md 的纪律）
    let p = Peripherals::take().unwrap();

    // 心跳灯：门控/退模拟/方向三步在 board::Led::new 里（第 2 章工程化）
    let mut led = Led::new(&p);

    defmt::info!("ch03 RTT hello — 复位默认时钟 HSI 4 MHz");

    loop {
        // M0+ 没有原子"读-改-写"指令，core 的 fetch_add 在 thumbv6m 上
        // 不存在——只能 load/store 三步走；单核且尚未开中断，安全。
        let n = SEQ.load(Ordering::Relaxed) + 1;
        SEQ.store(n, Ordering::Relaxed);
        defmt::info!("heartbeat #{=u32}", n);
        led.toggle(); // 灯与日志打点同步，双通道互为佐证
        delay_ms(500);
    }
}
```

两个 `use … as _` 是本书例程的惯用手法：`defmt_rtt` 和 `panic_probe`
的功能靠链接符号（全局 logger、panic handler）生效，不需要显式调用
任何 API，`as _` 把"引入它"的意图写在最显眼的地方且不产生未使用警告。
`info!` 的 `#{=u32}` 是 defmt 的类型化格式：`=u32` 声明参数按 u32 编码，
主机端据此还原。

延时与心跳灯来自第 2 章工程化小节的 `board` 封装：`board::delay_ms(500)`
是 SysTick 忙等（毫秒级标定，替代本章旧版的数循环延时），
`board::Led::new` 三步完成点灯前置。日志打点与 LED 翻转同步——板上
首跑时，"灯在闪而日志没滚出来"即可判定问题出在 RTT 链路而非主循环。

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch03_rtt -p cw32-book-pac
```

## 运行：一条命令烧录 + 收日志

probe-rs 的 `run` 子命令把"烧录、复位、附着 RTT、解码 defmt"串成一
条命令（探针与芯片参数见第 1 章，完整速查见附录 D）：

```bash
probe-rs run --probe 0483:3748 --protocol swd --chip CW32L012x8 \
    target/thumbv6m-none-eabi/release/examples/ch03_rtt
```

固件周期性打点，终端按行滚出解码后的日志；每行开头是时间戳提供者
返回的值（本例即日志序号）。预期形态大致是：

```text
      1 INFO ch03 RTT hello — 复位默认时钟 HSI 4 MHz
      2 INFO heartbeat #1
      3 INFO heartbeat #2
      …（Ctrl-C 退出）
```

（具体列宽、颜色与时间戳渲染由 probe-rs 版本决定，以实际输出为准；
本例程未实机核对过这批字节，板上首跑请对照"板上验证"一节的核对项。）

`probe-rs run` 会在 main 返回后自动退出——本例程 `main` 永不返回，
所以看够了用 Ctrl-C 断开，固件继续在板上跑。若只想附着不重新烧录，
用 `probe-rs attach`（参数相同）。

## defmt 还是 rprintln？

RTT 上跑日志还有另一条路：`rtt-target` 的 `rprintln!`——原样输出 UTF-8
文本，主机端终端直接可读，不需要解码、不需要 ELF。

两条路的取舍：

| | defmt（本章） | rprintln（rtt-target） |
|---|---|---|
| 固件开销 | 小：格式串 intern 进 ELF，帧是紧凑二进制 | 大：每条日志带完整文本 |
| 主机端 | 需要解码（probe-rs 内置） | 直接可读 |
| 级别过滤 | 编译期，`DEFMT_LOG` 控制 | 无，打印即发生 |
| panic 报告 | `panic-probe` 打印位置后停机 | 需自己接 |
| 与 rtt-target 共存 | 不能（RTT 控制块唯一） | —— |

嵌入式 Rust 生态的默认选择是 defmt（embassy 全系用它），本书后续章节
的日志也全部走 defmt。`rprintln` 适合"就想要个 printf"的一次性调试。

## 板上验证

**本例程本身：仅编译未上板**（thumbv6m-none-eabi release 构建通过；
`nm` 可确认 `_SEGGER_RTT` 控制块、`_defmt_timestamp`、`.defmt` 元数据
段均已落位，向量表正常）。

依赖组合（defmt 1 + defmt-rtt 0.4 + panic-probe(print-defmt)）与
embassy-cw32 的 `rtt_hello` 完全一致；需要说明的是，`rtt_hello` 的
提交记录写明的验证级别是 **"release link clean"**（thumbv6m release
链接通过），亦未宣称上板——RTT 链路在本生态中尚无实机日志记录可引，
首次上板请按下表逐项核对，结果欢迎回填到本书 issues。

首次上板核对项：

1. `probe-rs list` 能看到探针；`probe-rs run` 不报
   `RTT control block not found`（控制块在 SRAM，若被链接器优化请检查
   `_SEGGER_RTT` 符号是否保留）；
2. 输出按行滚出 `INFO` 级别日志，序号连续递增；若出现乱码/缺行，先降
   `DEFMT_RTT_BUFFER_SIZE` 到 512 再试（缓冲溢出截断是 RTT 最常见
   症状——defmt-rtt 默认非阻塞截断模式）；
3. Ctrl-C 断开后，核对固件是否仍在运行（预期：继续跑，PC13 心跳灯
   以 1 Hz 闪烁——它与日志打点同步翻转）。灯在闪而日志没滚出来，
   问题在 RTT 链路，回到核对项 1、2；灯也不闪，先查 GPIOC 门控与
   ANALOG（第 2 章的两个静默坑）。

## 常见坑

- **链接报 `undefined symbol: _critical_section_1_0_acquire`**：没有
  critical-section 实现。按本章例程 `set_impl!` 一个；一个固件只能有
  一个实现。
- **`timestamp!` 用了两次**：其标记符号（`__DEFMT_MARKER_TIMESTAMP_…`）
  在整个 crate 图只能定义一次，重复定义在链接期报重复符号——时间戳
  提供者要么放最终固件 crate，要么只放其中一个依赖里。
- **在 thumbv6m 上用 `fetch_add` 等 RMW 原子操作**：编译报 method not
  found——M0+ 没有原子读改写指令，`core` 只提供 `load`/`store`。
  需要完整原子语义时用 `critical_section::with` 包住普通读写。
- **`defmt` 与 `rtt-target` 同时引入**：RTT 控制块冲突，二选一。
- **日志"丢了尾巴"**：defmt-rtt 默认缓冲满即截断（不阻塞固件），
  高频日志请加大 `DEFMT_RTT_BUFFER_SIZE` 或降低打印频率。
- **时间戳想显示成秒/毫秒**：单位是显示端约定，设备端只管给数——
  格式串里写什么提示，probe-rs 就按什么渲染（`{=u32:ms}` 之类）。
