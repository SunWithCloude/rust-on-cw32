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

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
