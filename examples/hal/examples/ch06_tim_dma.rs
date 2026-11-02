//! 第 6 章例程（上）：BTIM1 定时中断闪灯 + DMA 存储器搬运（零接线）。
//!
//! 段 A（定时器）：BTIM1 溢出中断 2Hz，ISR 里 TOG 翻转 PC13，主循环
//! `wfi` 睡眠——与 cw32l012-hal examples/timer_blink.rs 同源。
//! 段 B（DMA）：`Dma::mem_to_mem` 把 16 个 u32 从 SRC 搬进 DST 并逐字
//! 校验，结果落在 `DMA_OK`/`DMA_BAD` 静态量（调试器可读）——搬运路径
//! 与 examples/dma_demo.rs 的 P1 相同（原例为 64 字版）。
//!
//! 构建后现象：PC13 电平每 500ms 翻转一次（溢出 2Hz）= 中断路径活着；
//! DMA 结果需接调试器读静态量。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch06_tim_dma -p cw32-book-hal
//! 状态：同源已验证——两段分别对应 TESTING.md 实测通过表的 timer_blink
//!       （PC13 翻转序列 0,0,1,0,0,1,1,0）与 dma_demo（P1_OK=64、
//!       P1_BAD=0xFFFF_FFFF）条目；本组合文件本身仅编译未上板。

#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, write_volatile};

use cw32l012_hal::{
    btim::Btim,
    dma::Dma,
    gpio::{Level, Pin, PortId},
    pac,
    rcc::Rcc,
    runtime, Peripherals,
};

/// 中断次数计数（调试器可观察）
static mut ISR_COUNT: u32 = 0;
/// DMA 源数据（编译期生成）
static SRC: [u32; 16] = {
    let mut a = [0u32; 16];
    let mut i = 0;
    while i < 16 {
        a[i] = 0xA5A5_0000 ^ (i as u32);
        i += 1;
    }
    a
};
/// DMA 校验：逐字一致的元素数（应 = 16）
static mut DMA_OK: u32 = 0;
/// DMA 校验：首个不匹配下标（0xFFFF_FFFF = 全部一致）
static mut DMA_BAD: u32 = 0xFFFF_FFFF;

#[unsafe(no_mangle)]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };

    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks();

    let _led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);

    // ---- 段 B：DMA 存储器→存储器（先搬运，后开中断）----
    {
        let mut dma = Dma::new(p.dma, &mut rcc);
        let mut dst = [0u32; 16];
        // 阻塞搬完（软件触发、32 位宽、自动选通道 3）；即使 Err 也不提前
        // 退出，让下面的逐字校验把结果如实记进 DMA_OK/DMA_BAD
        let _ = dma.mem_to_mem(&SRC, &mut dst);
        let mut ok = 0u32;
        let mut bad = 0xFFFF_FFFFu32;
        for i in 0..16 {
            if dst[i] == SRC[i] {
                ok += 1;
            } else if bad == 0xFFFF_FFFF {
                bad = i as u32;
            }
        }
        unsafe {
            write_volatile(addr_of_mut!(DMA_OK), ok);
            write_volatile(addr_of_mut!(DMA_BAD), bad);
        }
    }

    // ---- 段 A：BTIM1 @2Hz 更新中断闪灯 ----
    let mut tim = Btim::new(p.btim1, &mut rcc);
    match tim.start_periodic(clocks.pclk_hz, 2) {
        Ok(_) => {}
        Err(_) => loop {
            core::hint::spin_loop();
        },
    }
    tim.enable_update_irq(true);

    // NVIC 层使能 BTIM1（IRQ20），然后全局开中断
    unsafe {
        runtime::Interrupt::Btim1.enable();
        core::arch::asm!("cpsie i");
    }

    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

// BTIM1 更新中断：清标志 → 翻转 PC13（TOG 原子写）→ 计数
cw32l012_hal::interrupt!("BTIM1_irq", btim1_isr, {
    unsafe {
        let p = Peripherals::steal();
        p.btim1.icr.write(pac::btim::ICR_CLEAR_UIF);
        p.gpioc.tog.write(1 << 13);
        ISR_COUNT += 1;
    }
});

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
