# 第 12 章 DMA

这一章你会学到：

- L012 DMA 的 4 通道模型与"数据块 × 数据块大小"的传输计数方式（§8）；
- 软件触发与硬件触发（`hs` 触发源表 8-8）的区别；
- `Dma::mem_to_mem` 最小回路的写法与四个实机验证过的坑；
- UART/SPI/ADC/DAC 各自的 DMA 变体长什么样（指向对应例程，不重复展开）。

## 通道模型

L012 的 DMA 控制器（§8，基地址 0x4002_0000）有 **4 个通道**，同一寄存器
块内按 0x20 字节排布，每通道一套 CSR/CNT/SRC/DST/TRIG/CCNT。数字越小
优先级越高（§8.5.4）。一次传输要配的东西在 §8.4.1 步骤 1~14，对应驱动
的 `Config` 字段：

- **模式**（CSR.TRANS，表 8-1）：BULK 一口气搬完不让总线；BLOCK 每搬完
  一个数据块插入间隙，允许 CPU/高优先级通道抢占；
- **触发**（TRIG.TYPE + TRIG.HARDSRC）：软件触发由 `Channel::start()` 写
  SOFTSRC 启动；硬件触发由外设事件启动，源编码查 **表 8-8**——驱动把
  整张表收编成 `pac::dma::hs` 常量（`UART1_RX=0x00`、`ADC1_SEQ=0x0C`、
  `DAC1=0x10`、`GTIM1_UPD=0x1A`……）；
- **位宽**（CSR.SIZE）：8/16/32 位，**同时决定地址自增量 1/2/4 字节**；
- **计数**（CNT）：`CNT` = 数据块个数（1..=65535），`REPEAT` = 每个数据
  块的元素数（1..=15，建议 1）。

两条 NVIC 线服务四个通道：**IRQ9 `DMA12`（通道 1/2 共享）、IRQ10
`DMA34`（通道 3/4 共享）**。`Channel::wait().await` 等 TC/TE（传输完成/
错误）：等待期间才开本通道的 TCIE/TEIE，中断唤醒后立即压掉——防风暴
纪律贯穿全书。**两条线都必须绑定**到 `Dma12Irq`/`Dma34Irq`（`Dma::new`
的 `_irqs` 参数在编译期强制）。

## mem_to_mem：最小回路

`Dma::mem_to_mem(&src, &mut dst)` 是最短的 DMA 体验：自动占**通道 3**
（把 1/2 留给实时外设），源/目的自增、BULK、软件触发，元素类型决定位宽
（u8/u16/u32 → 8/16/32 位）。完成后 `wait()` 返回 `Result<(), DmaError>`，
错误按表 8-2 的 STATUS 编码分类（地址越界/中止/源错/目的错）。

外设方向的便捷入口同形：`periph_to_mem`（外设→内存，占通道 1，源固定
不自增）、`mem_to_periph`（内存→外设，占通道 2，目的固定）。它们只把
DMA 侧配好并挂起等请求——"外设侧怎么发请求"（如 ADC 的 IER.DMAEOS、
UART 的 CR2 位、SPI 的 CR2.DMARX/DMATX）属于各外设的章节。

## 例程

`examples/embassy/examples/ch12_dma_m2m.rs`：64 个 u32 从 FLASH 常量表
搬到 SRAM（32 位宽、BULK、软件触发），逐字比对并记录结果；每 500 ms
重跑一轮。注意每轮都重新走 `mem_to_mem`——它内部会重新 `configure`，
原因见下面坑 2。

```rust
//! 第 12 章例程：DMA 存储器→存储器搬运（64 × u32，零接线）。
//!
//! 64 个 u32 从 FLASH 常量表 `SRC` 搬到 SRAM `DST`（32 位宽、BULK、软件
//! 触发，`Dma::mem_to_mem` 占通道 3），逐字比对并记录首个失配下标；随后
//! 每 500ms 重跑一轮（每次都重新 configure——`CNT.REPEAT` 每轮传输后被
//! 硬件清零，§8.5.2），ROUNDS 计数即异步 wait 路径的心跳。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch12_dma_m2m -p cw32-book-embassy
//! 状态：仅编译未上板（embassy 版 dma_mem2mem 同状态）；配方与阻塞版
//!       dma_demo P1 同源且已实机验证（P1_OK=64、DST 图样逐字一致，
//!       2026-10-23，见 hal TESTING.md）。首次上板核对点见第 12 章正文。

#![no_std]
#![no_main]

use embassy_executor::main;
use embassy_time::Timer;

use cw32_book_embassy as _; // 继承 lib.rs 的 panic handler（panic-probe 同款机制）
use embassy_cw32::{Config, Peripherals};

embassy_cw32::bind_interrupts!(struct Irqs {
    DMA12 => embassy_cw32::dma::Dma12Irq;
    DMA34 => embassy_cw32::dma::Dma34Irq;
});

/// 源数据（FLASH 常量表，编译期生成的校验图样）
static SRC: [u32; 64] = {
    let mut a = [0u32; 64];
    let mut i = 0;
    while i < 64 {
        a[i] = 0xA5A5_0000 ^ ((i as u32) * 0x0001_0101);
        i += 1;
    }
    a
};

/// 目的缓冲（DMA 写入；probe-rs 读 0x20000000 起的 256 字节核对图样）
#[used]
#[unsafe(no_mangle)]
pub static mut DST: [u32; 64] = [0; 64];
/// 比对通过的元素数（应为 64）
#[used]
#[unsafe(no_mangle)]
pub static mut P1_OK: u32 = 0;
/// 首个失配下标（0xFFFF_FFFF = 全部一致）
#[used]
#[unsafe(no_mangle)]
pub static mut P1_BAD: u32 = 0;
/// 错误码：0=无、1=DMA 错误（STATUS 解码见 `Channel::check`，正常不应出现）
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;
/// 完成的轮数（心跳：每轮一次 DMA TC 中断唤醒）
#[used]
#[unsafe(no_mangle)]
pub static mut ROUNDS: u32 = 0;

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default());
    let p = Peripherals::take().unwrap();
    let mut dma = embassy_cw32::dma::Dma::new(p.dma, Irqs);

    loop {
        // SRC 在 FLASH、DST 在 SRAM：阻塞版 dma_demo P1 的同款通路
        let dst = unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(DST) as *mut u32, SRC.len())
        };
        match dma.mem_to_mem(&SRC, dst).await {
            Ok(()) => unsafe { ERR = 0 },
            Err(_) => unsafe { ERR = 1 },
        }

        // 逐字比对（DMA 已 TC 完成；单核 M0+ 无缓存，直读 SRAM 即最新值）
        unsafe {
            let mut ok = 0u32;
            let mut bad = 0xFFFF_FFFFu32;
            for i in 0..SRC.len() {
                if DST[i] == SRC[i] {
                    ok += 1;
                } else if bad == 0xFFFF_FFFF {
                    bad = i as u32;
                }
            }
            P1_OK = ok;
            P1_BAD = bad;
            ROUNDS += 1;
        }

        Timer::after_millis(500).await;
    }
}
```

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch12_dma_m2m -p cw32-book-embassy
```

## 板上验证

**状态：仅编译未上板。** 本例程与 embassy-cw32 仓的 `dma_mem2mem` 示例
同源；寄存器配方与阻塞版 `dma_demo` P1 相位逐项同源——该配方已于
2026-10-23 实机验证（hal 仓 TESTING.md：P1_OK=64、P1_BAD=0xFFFF_FFFF、
DST 图样 `A5A5_0000 ^ i×0x0101_0101` 逐字一致），未上板的增量是 embassy
的异步 wait 路径。

首次上板核对点：

- `P1_OK=64`、`P1_BAD=0xFFFF_FFFF`、`ERR=0`，`DST`（SRAM 0x20000000 起
  256 字节）与 SRC 图样逐字一致；
- `ROUNDS` 每 500 ms +1——每轮一次 TC 中断唤醒，证明异步 wait 不是
  空转。

## UART/SPI 的 DMA 变体

DMA 的价值在外设流式收发。embassy-cw32 已有两类现成变体，本书不重复
展开，正文中只给出入口：

- **UART**：`Uart::read_dma` / `Uart::write_dma`（单次全量搬运，触发源
  `hs::UART1_RX/TX`，UART 请求使能在 CR2[6]/[7]）——embassy 仓
  `examples/uart_dma.rs` 是完整回显示例。⚠ 语义级实现待实板验证
  （阻塞版无 UART DMA 配方）。
- **SPI**：`Spi::write_dma` / `Spi::transfer_dma`（双通道全双工，触发源
  `hs::SPI1_RX/TX`，请求使能在 CR2.DMARX/DMATX，§22.7）——embassy 仓
  `examples/spi_dma.rs` 零接线可跑、杜邦线短接 MOSI↔MISO 可验回环。
  ⚠ 语义级实现待实板验证。

另有 ADC 序列 DMA（第 11 章 `read_sequence_dma`，"推广路径待板核对"）
与 DAC 波形 DMA（第 11 章 `write_dma`）两个联动场景。

## 常见坑

以下四条全部来自阻塞版 `dma_demo` 的实机验证记录（hal 仓 TESTING.md），
寄存器行为与 embassy 版一致：

- **`ICR` 是 R1W0**：写 1 无动作、**写 0 才清标志**（复位值全 1）。驱动
  已封装（`clear_flags`），自己摸寄存器时极易踩反。
- **`CNT.REPEAT` 每次传输后被硬件清 0**（§8.5.2）：想"再传一次"必须
  重新 `configure` 整个通道，不能只改 CNT。
- **软件触发 `SOFTSRC` 传输完成后自动清零**：软件触发模式下"再传一次"
  要重新写 SOFTSRC；硬件触发则只需外设继续发请求。
- **地址自增量由位宽决定**：ADC 的多通道结果寄存器 RESULT0..7 步进是
  4 字节（表 25-7），配 16 位宽的自增量 2 字节对不上——要么配 32 位宽，
  要么用 `REPEAT` 整块搬（第 11 章的 `read_sequence_dma` 用后者）。
- **源/目的地址不能是 DMA/FLASH/RAM 控制器自己的寄存器**（§8.8.6 注），
  否则 STATUS 报地址越界类错误。
