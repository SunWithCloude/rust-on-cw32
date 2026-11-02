# 第 14 章 I2C 与 SPI

这一章你会学到：

- L012 命令 FIFO 式 I2C 外设的模型，以及它与经典家族（F030）8051 状态机
  式 I2C 的代差（§23）；
- `I2c::new_i2c1` 的构造参数与扫描总线的最小写法；
- SPI 主模式的 `Spi` 驱动：软件片选、`SpiBus` 异步接口（§22）。

## I2C：两代 IP，两个世界

CW32 两代家族的 I2C 外设是**两套完全不同的 IP**，寄存器面没有一处同构：

- **L012（命令 FIFO 式，§23）**：主机往 `MTDR` 写"命令 + 数据"——
  START+地址、TX 字节、RXCOUNT（要收几个）、STOP 各是一条命令；进度与
  错误从 `MISR` 读（TXE/RXNE/PACKET/NACK/ARBI/FIFO/PINLOW），经 MIER
  中断 + waker 异步化。时序参数（PRESCALE/SETHOLD/CLKHI/CLKLO/DATAVD）
  由驱动按 I2C 规范推导，已对照手册表 23-4 的官方示例校准。
- **经典家族 F030（8051 状态机式）**：CR 置 STA/STO、DR 装字节，SI 置位
  时读 STAT 状态码（0x08/0x10/0x18/…/0x58），一次 SI 一个动作——写法
  直接继承 8051 的 I2C 时代。

`embassy-cw32` 的 `I2c` 把两者统一到同一个 `embedded-hal-async` 接口下
（`write`/`read`/`transaction`），但**事务语义有差别**：L012 版的每个
Operation 独立执行 START…STOP（`Write` 后跟 `Read` 不做重复起始）；F030
版整个 transaction 单次 START、切方向时发重复起始（repeated-START），
更强，适合"写寄存器指针 + 读数据"类器件。用到跨方向事务时记得查对应
家族驱动的文档。

引脚映射（L012，手册表 9-2）：I2C1 的 SCL = PA04/PA09、SDA = PA05/PA10，
均 AF2；开漏电气由外设的 PINCFG=OD_IO 模式处理，**上拉电阻要外接**
（4.7 kΩ × 2 到 VDD）。

## 例程：总线扫描

`examples/embassy/examples/ch14_i2c_scan.rs`：i2cdetect 式探测——对
0x08..0x78 的每个地址发 START+ADDR+W（空数据写），ACK 即命中、NACK 即
跳过。命中地址记入静态数组（`HITS`/`MISS`，probe-rs 可读），LED 给出
目视判定：有命中 900 ms 亮 / 100 ms 灭，空总线慢闪。

```rust
//! 第 14 章例程：I2C1 总线扫描（L012）。
//!
//! I2C1 主机：PA09=SCL / PA10=SDA（AF2，手册表 9-2；I2C1 另一组
//! PA04/PA05 同为 AF2）。总线需外接 4.7kΩ 上拉到 VDD，并与板卡共地。
//! 周期性扫描地址 0x08..0x78（i2cdetect 惯例）：对每个地址发
//! START+ADDR+W（空数据探测），ACK 即命中、NACK 即跳过。命中地址记入
//! 静态数组（宿主可经 probe-rs 读内存核对）。
//!
//! LED（PC13）目视判定：有命中 900ms 亮/100ms 灭；无命中 500ms 翻转
//! （慢闪）。板上未接任何从器件时全地址 NACK，慢闪即为正确行为。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch14_i2c_scan -p cw32-book-embassy
//! 状态：仅编译未上板（L012 命令 FIFO 式 I2C 驱动语义待板核对）；扫描
//!       例程骨架与 embassy 版 i2c_scan_f030 逐行同源，后者已在 CW32F030
//!       实板验证（空总线 112/112 NACK，见 embassy-cw32 CHANGELOG）。
//!       首次上板核对点见第 14 章正文。

#![no_std]
#![no_main]

use core::cell::RefCell;

use critical_section::Mutex;
use cw32_book_embassy as _; // 继承 lib.rs 的 panic handler（panic-probe 同款机制）
use embassy_executor::main;
use embassy_time::Timer;
use embedded_hal_async::i2c::I2c as _;

use embassy_cw32::{
    Config, Peripherals, SYSCTRL_KEY,
    gpio::{Level, Output, Pin},
    i2c::{self, I2c},
};

embassy_cw32::bind_interrupts!(struct Irqs {
    I2C1 => i2c::I2c1Irq;
});

/// 扫描地址范围（避开 0x00-0x07 保留段与 0x78 起的 10 位地址前缀）
const SCAN_RANGE: core::ops::Range<u8> = 0x08..0x78;
/// 命中地址最多记录条数（超出只计总数）
const MAX_HITS: usize = 16;

/// 扫描命中记录（静态存放，便于宿主经 probe-rs 读内存核对）：
/// `[0]` = 命中个数，其后依次为地址，空位保持 0。
static HITS: Mutex<RefCell<[u8; 1 + MAX_HITS]>> = Mutex::new(RefCell::new([0; 1 + MAX_HITS]));

/// 未命中原因计数（上板验证用）：`[0]`=NACK（正常无器件）、`[1]`=总线
/// 异常（FIFO/PINLOW）、`[2]`=仲裁失败。空总线上应全为 NACK。
/// `#[used]`：固件只写不读，防止链接器把符号裁掉（宿主要读内存）。
#[used]
static MISS: Mutex<RefCell<[u8; 3]>> = Mutex::new(RefCell::new([0; 3]));

/// 把本轮命中与未命中原因写入静态记录。
fn record_round(hits: &[u8], miss: [u8; 3]) {
    critical_section::with(|cs| {
        let mut slot = HITS.borrow_ref_mut(cs);
        slot[0] = hits.len() as u8;
        for (dst, src) in slot[1..].iter_mut().zip(hits) {
            *dst = *src;
        }
        *MISS.borrow_ref_mut(cs) = miss;
    });
}

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();
    // GPIOA（I2C 引脚）+ GPIOC（LED）同写一个 AHBEN：KEY 整字写，分两次
    // 写会互相顶掉使能位（A=4/C=6）
    p.sysctrl.ahben.write(SYSCTRL_KEY | (1 << 4) | (1 << 6));

    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    // 引脚复用/开漏电气由驱动内部处理（MCR2.PINCFG=OD_IO）；上拉电阻
    // 需外接（4.7kΩ × 2 到 VDD）
    let mut i2c = I2c::new_i2c1(
        p.i2c1,
        Pin::new(p.gpioa, 9),  // SCL，AF2（手册表 9-2）
        Pin::new(p.gpioa, 10), // SDA，AF2
        2,
        Irqs,
        i2c::Config::default(), // 默认 = 标准模式 100kHz
    );

    loop {
        // 一轮扫描：对每个地址发 START+ADDR+W，ACK=命中 / NACK=跳过
        let mut hits = [0u8; MAX_HITS];
        let mut n = 0usize;
        let mut miss = [0u8; 3];
        for addr in SCAN_RANGE {
            match i2c.write(addr, &[]).await {
                Ok(()) => {
                    if n < MAX_HITS {
                        hits[n] = addr;
                    }
                    n += 1;
                }
                Err(i2c::I2cError::Nack) => miss[0] = miss[0].saturating_add(1),
                Err(i2c::I2cError::Arbitration) => miss[2] = miss[2].saturating_add(1),
                Err(_) => miss[1] = miss[1].saturating_add(1), // Fifo / PinLow
            }
        }
        record_round(&hits[..n.min(MAX_HITS)], miss);

        if n > 0 {
            // 总线上有器件：常亮间隔闪（亮灭占比 9:1 双态可辨）
            led.set_low();
            Timer::after_millis(900).await;
            led.set_high();
            Timer::after_millis(100).await;
        } else {
            // 总线空：慢闪（1s 周期翻转）
            led.toggle();
            Timer::after_millis(500).await;
        }
    }
}
```

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch14_i2c_scan -p cw32-book-embassy
```

## SPI：主模式与软件片选

L012 的 SPI1/2/3（§22）是常规全双工主/从外设：CR1 配 MSTR/CPOL/CPHA/
位序，BR 位域（7 位 @CR1[24..31]）决定 SCK = PCLK/(BR+1)，EN 在 CR2[0]
——且 CR1 必须在 EN=0 时写，驱动构造函数已按此顺序处理。L012 的 SPI1
常用引脚：SCK = PA05/PA10、MOSI = PA07/PA12、MISO = PA06/PA11，均 AF3
（表 9-2；注意 PA04/PA05 同时是 I2C1 与 DAC_OUT 的复用脚，板上多外设
共存时先对表 9-2 分配引脚）。

`embassy-cw32::spi::Spi`（构造 `Spi::new_spi1(p.spi1, sck, mosi, miso,
af, Irqs, Config)`，绑定 IRQ25 `SPI1`）实现 `embedded-hal-async` 的
`SpiBus` trait：`write` / `read` / `transfer`（全双工同长收发）/
`transfer_in_place` / `flush`。**片选是软件的**：驱动不管 CS，你在传输
前把一根普通 GPIO 输出拉低、`flush()`（等 BUSY 排空）之后拉高——阻塞
版 `tft_hw_spi` 例程驱动 ST7789 屏幕用的正是这套时序（24 MHz 实机验证
通过）。每个从设备一根 CS，多设备共享总线时轮流拉低即可。

DMA 变体（`write_dma`/`transfer_dma`，CR2.DMARX/DMATX，§22.7）已在第
12 章提过：embassy 仓 `examples/spi_dma.rs` 零接线可跑；经典家族的
SpiBus 回环示例见同仓 `examples/spi_loopback_f030.rs`。两者均属语义级
实现待实板验证，使用前读一下例程头注释的状态标注。

## 板上验证

**状态：仅编译未上板（L012 路径）。** 本例程是 embassy-cw32 仓
`i2c_scan_f030` 的 L012 移植：**例程骨架逐行同源**（扫描范围、命中/
未命中记录、LED 判定逻辑一致），且**F030 版已在 CW32F030 实板验证**——
空总线 112/112 地址全部 NACK（embassy-cw32 CHANGELOG 多家族化条目，
2026-10-24）。但两代 I2C 是不同 IP、L012 驱动是另一套实现，L012 侧
尚未上板，不能按"同源已验证"计。

首次上板核对点：

- 空总线：`MISS[0]=112`（全 NACK）、`MISS[1]=MISS[2]=0`，LED 慢闪；
  若 `MISS[1]` 增长说明总线异常（FIFO/PINLOW——先查上拉电阻与共地）；
- 接一只已知地址的从器件（如 0x68 的 MPU6050）：`HITS[0]` 应等于器件
  地址，LED 呈常亮间隔闪；
- 逻辑分析仪看 SCL：标准模式 100 kHz 方波；地址字节后第 9 个时钟看
  ACK/NACK 电平是否与分析仪解码一致。

## 常见坑

- **忘接上拉电阻**：I2C 是开漏总线，没有上拉时 SCL/SDA 停在低电平，
  L012 驱动会报 `I2cError::PinLow`（总线卡死）——先量两根线的静态
  电平再查软件。
- **引脚复用撞车**：PA04/PA05 同时是 I2C1 与 DAC_OUT1/2 的候选脚、
  PA05/PA06/PA07 又是 SPI1——L012 引脚少，多外设并存时逐项对表 9-2，
  必要时把 I2C 挪到 PA09/PA10。
- **L012 事务无重复起始**：`transaction(addr, &[Write, Read])` 在 L012
  上是两次独立事务（STOP 后再 START）；EEPROM 随机读、传感器保持
  寄存器这类需要 repeated-START 的器件，在 L012 上要么拆成两次操作
  （确认器件接受 STOP 之间不重置地址指针），要么换 F030 家族。
- **SPI 帧间 CS 时序**：拉高 CS 前必须 `flush()` 等移位寄存器排空，
  直接拉高会截掉最后一字节；反过来传输中途拉高 CS 则从设备丢帧。
- **CPOL/CPHA 与器件手册对表**：Mode 0 是最常见默认，但屏幕/Flash/
  传感器各有例外——波形错了先查模式再查别的。
