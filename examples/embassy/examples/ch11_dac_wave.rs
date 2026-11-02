//! 第 11 章例程：DAC1 DMA 三角波输出（GTIM1 TRGO 供拍，零接线可跑）。
//!
//! 触发链：GTIM1 配 1kHz 更新 + TRGO（CR2.MMS=update）→ DAC1 触发源
//! TSEL=GTIM1_Trgo → 每次触发播一个样点并请求 DMA 预载下一个
//! （`write_dma`，DMA 通道 2）。32 点三角波 = 32ms 一轮，PC13 每轮翻转。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch11_dac_wave -p cw32-book-embassy
//! 状态：仅编译未上板（embassy 版 dac_wave 同状态；DAC 的 DMA 请求搬运
//!       语义出自手册 §26.7，但 TRGO 供拍 × 请求线 × 末样点补播时序未上
//!       板核对）。首次上板核对点见第 11 章正文。

#![no_std]
#![no_main]

use embassy_executor::main;
use embassy_time::{Duration, Timer, with_timeout};

use embassy_cw32::{
    Config, Peripherals,
    dac::{Dac, DacChannel, DacError, DacTrig},
    gpio::{Level, Output, Pin},
    pac,
};

embassy_cw32::bind_interrupts!(struct Irqs {
    DMA12 => embassy_cw32::dma::Dma12Irq;
    DMA34 => embassy_cw32::dma::Dma34Irq;
});

/// 32 点三角波（0→4095→0，上坡 16 点、下坡镜像；1kHz 供拍下 32ms 一轮）
static TRIANGLE: [u16; 32] = {
    let mut a = [0u16; 32];
    let mut i = 0;
    while i < 32 {
        let k = if i < 16 { i } else { 31 - i };
        a[i] = (k as u32 * 273) as u16; // 峰值 15×273=4095
        i += 1;
    }
    a
};

/// 完成的整段播放轮数（心跳：每轮一次 DMA TC 唤醒）
#[used]
#[unsafe(no_mangle)]
pub static mut ROUNDS: u32 = 0;
/// 错误码：0=无 1=超时（触发源停拍？） 2=DMA 下溢 3=DMA 通道错误
/// 4=值越界 5=缓冲长度非法
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;
/// 最后一轮播放完的 DOR1（预期恒 0 = 三角波终点，末样点补播生效）
#[used]
#[unsafe(no_mangle)]
pub static mut LAST_DOR: u32 = 0;

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // GPIOC 时钟 + PC13 LED 心跳
    embassy_cw32::sysctrl_enable_gpioc(p.sysctrl);
    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    // ---- GTIM1 → TRGO 供拍（1kHz 更新；表 26-1 TSEL=0100 = GTIM1_Trgo）----
    // APBEN1.GTIM1（bit6，KEY 读改写——保留其它外设使能位）
    p.sysctrl.apben1.modify(|v| {
        pac::sysctrl::KEY | (v & 0xFFFF) | pac::sysctrl::APBEN1_GTIM1
    });
    let gtim1 = p.gtim1;
    gtim1.cr1.write(0); // 停止计数以便配置
    // PSC=3、ARR=999：PCLK 4MHz /(3+1)/(999+1) = 1kHz 更新
    gtim1.psc.write(3);
    gtim1.arr.write(999);
    // CR2.MMS[6:4]=010（update → TRGO；PAC 未收编该位域故写裸值）
    gtim1.cr2.write(2 << 4);
    gtim1.egr.write(pac::gtim::EGR_UG); // 重装影子寄存器
    gtim1.cr1.write(pac::gtim::CR1_EN); // 跑起来，开始供拍

    // ---- DAC + DMA 波形链（默认不使能引脚输出：PA4 与 SPI1 引脚冲突；
    //      需要示波器看波形时补一行 dac.enable_pin1()，探头接 PA4）----
    let mut dac = Dac::new(p.dac);
    let mut dma = embassy_cw32::dma::Dma::new(p.dma, Irqs);
    dac.set_trigger(DacChannel::Ch1, DacTrig::Gtim1Trgo);

    loop {
        // 32 样点 @1kHz = 32ms/轮；2s 超时兜底（触发源停拍时不挂死）
        let r = with_timeout(
            Duration::from_millis(2000),
            dac.write_dma(&mut dma, DacChannel::Ch1, &TRIANGLE),
        )
        .await;
        match r {
            Ok(Ok(())) => unsafe {
                ERR = 0;
                LAST_DOR = dac.output1() as u32;
            },
            Ok(Err(DacError::DmaUnderrun)) => unsafe { ERR = 2 },
            Ok(Err(DacError::Dma(_))) => unsafe { ERR = 3 },
            Ok(Err(DacError::ValueOutOfRange)) => unsafe { ERR = 4 },
            Ok(Err(DacError::BadLength)) => unsafe { ERR = 5 },
            Err(_) => {
                // 超时：触发链没跑起来/请求没到——停通道收尾再重试
                dac.stop_dma(&mut dma, DacChannel::Ch1);
                unsafe { ERR = 1 }
            }
        }
        unsafe {
            ROUNDS += 1;
            led.toggle();
        }
        Timer::after_millis(468).await;
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
