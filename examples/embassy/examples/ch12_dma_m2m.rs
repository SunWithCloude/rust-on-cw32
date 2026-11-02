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
