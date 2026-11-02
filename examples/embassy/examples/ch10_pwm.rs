//! 第 10 章例程：GTIM1 单通道 PWM（PB04，1kHz，占空比 10%→90% 扫动）。
//!
//! 每个 PWM 周期用 `wait_update().await` 节拍（更新中断 UIF + waker），
//! PC13 每 500 个更新事件翻转一次（1kHz 节拍下 = 1Hz 心跳）。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch10_pwm -p cw32-book-embassy
//! 状态：仅编译未上板（embassy 版 timer_pwm 同状态）；寄存器配方与阻塞版
//!       gtim_pwm 同源且已实机验证（ARR=3999/PSC=0、CNT 铺满，见 hal
//!       TESTING.md）。首次上板核对点见第 10 章正文。

#![no_std]
#![no_main]

use embassy_executor::main;

use cw32_book_embassy as _; // 继承 lib.rs 的 panic handler（panic-probe 同款机制）
use embassy_cw32::{
    Config, Peripherals, SYSCTRL_KEY,
    gpio::{Level, Output, Pin},
    timer::{GtimPwm, PwmChannel, PwmConfig},
};

// wait_update 需要 GTIM1 更新中断（IRQ16）；纯 PWM 用法无需绑定任何中断
embassy_cw32::bind_interrupts!(struct Irqs {
    GTIM1 => embassy_cw32::timer::Gtim1Irq;
});

/// (ARR<<16 | PSC) 配置回读：1kHz@PCLK 4MHz 预期 0x0F9F_0000
#[used]
#[unsafe(no_mangle)]
pub static mut PERIOD: u32 = 0;
/// 占空比读回（万分比，由 CCR/ARR 反算）
#[used]
#[unsafe(no_mangle)]
pub static mut DUTY_RB: u16 = 0;
/// wait_update 完成次数（预期 ≈1000/s）
#[used]
#[unsafe(no_mangle)]
pub static mut UPDATES: u32 = 0;
/// 错误码：0=无、1=构造失败（频率超范围/占空比越界，本例常量不会触发）
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // GPIOB（PB04 PWM 脚）+ GPIOC（PC13 LED）同写一个 AHBEN：L012 带
    // KEY 写保护、整字写，分开两次写会互相顶掉使能位（B=5/C=6）
    p.sysctrl
        .ahben
        .write(SYSCTRL_KEY | (1 << 5) | (1 << 6));
    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    // PWM：PB04 = GTIM1_CH1（AF6，手册表 9-2）；1kHz、初始占空比 10%
    let mut pwm = match GtimPwm::new(
        p.gtim1,
        Pin::new(p.gpiob, 4),
        6,
        PwmConfig {
            channel: PwmChannel::Ch1,
            freq_hz: 1_000,
            duty_bp: 1_000, // 万分比：10000 = 100%
        },
    ) {
        Ok(t) => t,
        Err(_) => {
            unsafe { ERR = 1 };
            loop {
                core::hint::spin_loop();
            }
        }
    };

    let (arr, psc) = pwm.period_config();
    unsafe { PERIOD = (arr as u32) << 16 | psc as u32 };

    // 占空比 10%→90%→10% 循环扫动（每步 5%，每个 PWM 周期走一步）
    let mut duty: u16 = 1_000;
    let mut dir_up = true;
    loop {
        // 万分比直接控（ARPE 预装载 → 下一更新周期生效）
        pwm.set_duty_bp(duty).ok();
        // 等一个真实 PWM 周期（GTIM1 更新中断唤醒；返回时刻 = 周期边界）
        pwm.wait_update().await;
        unsafe {
            DUTY_RB = pwm.duty_bp();
            UPDATES += 1;
            if UPDATES % 500 == 0 {
                led.toggle(); // 1kHz 节拍下 500 次 = 1Hz 心跳
            }
        }
        if dir_up {
            duty += 500;
            if duty >= 9_000 {
                dir_up = false;
            }
        } else {
            duty = duty.saturating_sub(500);
            if duty <= 1_000 {
                dir_up = true;
            }
        }
    }
}
