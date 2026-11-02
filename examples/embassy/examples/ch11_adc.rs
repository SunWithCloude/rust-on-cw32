//! 第 11 章例程：ADC1 异步单次转换（外部通道 PA0/IN0 + 内部基准测 VDDA）。
//!
//! 零接线可跑：PA0 悬空读数不定（验证通路本身），VDDA 走 IN15 内部
//! 1.2V 基准 + 出厂校准，预期 ~3200-3300 mV。PC13 每轮翻转。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch11_adc -p cw32-book-embassy
//! 状态：仅编译未上板（embassy 版 adc_async 同状态）；VDDA/内部通道配方
//!       与阻塞版 adc_internal 同源且已实机验证（VDDA=3282mV，见 hal
//!       TESTING.md）。首次上板核对点见第 11 章正文。

#![no_std]
#![no_main]

use embassy_executor::main;
use embassy_time::Timer;

use embassy_cw32::{
    Config, Peripherals, SYSCTRL_KEY,
    adc::{Adc, AdcChannel, AdcClk, SampleCycles},
    gpio::{Level, Output, Pin},
};

embassy_cw32::bind_interrupts!(struct Irqs {
    ADC1 => embassy_cw32::adc::Adc1Irq;
});

/// 最近一轮结果：[0]=VDDA（mV），[1]=PA0 原始码（0..4095）。
/// 单任务独写 + probe-rs 只读，static mut 可接受。
#[used]
#[unsafe(no_mangle)]
pub static mut RESULTS: [u32; 2] = [0; 2];
/// 错误码：0=无 1=Timeout 2=BadCalibration 3=UnsupportedChannel
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;
/// 完成的轮数（心跳计数）
#[used]
#[unsafe(no_mangle)]
pub static mut SAMPLES: u32 = 0;

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // GPIOA（PA0 模拟脚）+ GPIOC（PC13 LED）同写一个 AHBEN：KEY 整字写，
    // 分两次写会互相顶掉使能位（A=4/C=6）
    p.sysctrl.ahben.write(SYSCTRL_KEY | (1 << 4) | (1 << 6));
    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    let mut adc = Adc::new_adc1(p.adc1, AdcClk::Div2, Irqs);
    // PA0 → 模拟模式（IN0；模拟外设要求 GPIOx_ANALOG.PINy=1）
    let _pa0 = Pin::new(p.gpioa, 0).into_analog();

    loop {
        // VDDA：内部 1.2V 基准通道 + 出厂校准（BGREN 稳定期由驱动处理）
        match adc.vdda_mv().await {
            Ok(mv) => unsafe { RESULTS[0] = mv },
            Err(e) => unsafe {
                ERR = match e {
                    embassy_cw32::adc::AdcError::Timeout => 1,
                    embassy_cw32::adc::AdcError::BadCalibration => 2,
                    _ => 3,
                }
            },
        }
        // PA0 外部通道（悬空时读数不定，接电位器/分压后随输入变化）
        if let Ok(raw) = adc.read(AdcChannel::In(0), SampleCycles::S24).await {
            unsafe { RESULTS[1] = raw as u32 };
        }
        unsafe {
            SAMPLES += 1;
            led.toggle();
        }
        Timer::after_millis(500).await;
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
