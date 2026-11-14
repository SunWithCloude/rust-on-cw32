//! 第 16 章例程(embassy 版):ATIM 三相互补 PWM + 死区 + 刹车。
//!
//! 与 embassy-cw32 examples/atim_motor.rs 同源(20kHz 三相互补、1µs 死区、
//! 三路占空比错 120° 三角斜坡、每个 PWM 周期 `wait_update().await` 节拍、
//! 刹车监控 + 显式恢复)。注意这不是正弦 SVPWM——示例只演示互补输出、
//! 死区与周期节拍,不做电机控制。
//!
//! # 接线(数据手册 V1.0 表 5-3,全部 AF7)
//!
//! | 信号 | 引脚 | 信号 | 引脚 |
//! |------|------|------|------|
//! | CH1  | PA8  | CH1N | PA7  |
//! | CH2  | PA9  | CH2N | PB0  |
//! | CH3  | PA10 | CH3N | PB1  |
//! | BK   | PA6  | (刹车输入,低电平有效,外部上拉) |
//!
//! 刹车路径:过流比较器(开漏)拉低 PA6 → 硬件**异步**清 MOE → 六路
//! 立即进空闲安全态(上/下管全关,不经软件/中断/死区逻辑)。ISR.BIF
//! 置位(`is_braked()`);刹车源活跃期间 BIF 清不掉(`clear_break()`
//! 返回 false)。排除故障 → `clear_break()` → `enable()` 手动恢复
//! (本例 `aoe=false`,恢复决策权留给上层)。
//!
//! # 验证方式(无示波器也可)
//!
//! - `PERIOD` 静态量记录 (PSC<<16 | ARR)——预期 PSC=0、ARR=199
//!   (20kHz @ PCLK 4MHz)
//! - `DTG` 记录 DTG 编码——1µs @ 4MHz = 4 tick,预期 0x04
//! - `UPDATES` ≈20000/s 线性增长(wait_update 真在等更新中断)
//! - `BRAKES`/`ENABLE_FAILS`:PA6 拉低再释放——BRAKES+1、
//!   ENABLE_FAILS+1(刹车压着时 enable() 置不上 MOE),释放并
//!   clear_break + enable 后恢复斜坡
//! - 示波器:PA8/PA7 互补方波 + 1µs 死区(交叠处两路同低),
//!   PA9/PB0、PA10/PB1 同理且错 120°
//!
//! 结果落 SRAM 静态量,`probe-rs read` 可读。
//!
//! 构建:cargo build --release --target thumbv6m-none-eabi --example ch16_atim_async -p cw32-book-embassy
//! 状态:仅编译未上板;驱动与阻塞版 atim 模块同源(阻塞版配方已在 hal
//!       仓 TESTING.md atim_deadtime 条目实机验证),embassy 版驱动
//!       compiles-not-run——上板前勿接真电机,先用逻辑分析仪小电压验证。

#![no_std]
#![no_main]

use embassy_executor::main;

use cw32_book_embassy as _; // 继承 lib.rs 的 panic handler(panic-probe 同款机制)
use embassy_cw32::{
    Config, Peripherals,
    atim::{AtimError, AtimPwm, AtimPwmConfig},
    gpio::Pin,
};

// wait_update 需要 ATIM 更新中断(IRQ13)
embassy_cw32::bind_interrupts!(struct Irqs {
    ATIM => embassy_cw32::atim::AtimIrq;
});

/// (PSC<<16 | ARR) 配置回读(预期 PSC=0、ARR=199)
#[used]
#[unsafe(no_mangle)]
pub static mut PERIOD: u32 = 0;
/// DTG 死区编码回读(预期 0x04 = 4 tick @ 4MHz 的 1µs)
#[used]
#[unsafe(no_mangle)]
pub static mut DTG: u32 = 0;
/// wait_update 完成次数(预期 ≈20000/s)
#[used]
#[unsafe(no_mangle)]
pub static mut UPDATES: u32 = 0;
/// 刹车进入次数(PA6 拉低事件)
#[used]
#[unsafe(no_mangle)]
pub static mut BRAKES: u32 = 0;
/// 刹车压着时 enable() 失败次数(MOE 置不上)
#[used]
#[unsafe(no_mangle)]
pub static mut ENABLE_FAILS: u32 = 0;
/// 构造错误码:0=无 1=FreqOutOfRange 2=DeadTimeTooLong 3=BadBreakConfig
/// 4=BadDuty
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;

/// 三角波占空比(万分比):0 → 10000 → 0,200 步一循环
fn ramp(step: u32) -> u16 {
    let s = step % 200;
    let v = if s < 100 { s * 100 } else { (200 - s) * 100 };
    (v.min(10_000)) as u16
}

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // GPIOA(bit4: PA6-PA10) + GPIOB(bit5: PB0/PB1) 端口时钟——KEY 保护
    // 寄存器整字写一次到位,分两次写会互相顶掉使能位(第 2 章同款坑)
    p.sysctrl
        .ahben
        .write(embassy_cw32::SYSCTRL_KEY | (1 << 4) | (1 << 5));

    // 三相互补 + 死区 + 刹车(引脚/AF 见文件头接线表)
    // 不用的通道传 None:APIT 端口类型无法从 None 推断,需显式标注
    let no_pin: Option<(Pin<embassy_cw32::Gpio>, u8)> = None;
    let pwm = AtimPwm::new(
        p.atim,
        AtimPwmConfig {
            freq_hz: 20_000,     // 20kHz 载波(4MHz PCLK 下 ARR=199)
            dead_time_ns: 1_000, // 1µs 死区(4 tick,向上取整)
            aoe: false,          // 刹车不自动恢复——clear_break + enable 显式来
        },
        Some((Pin::new(p.gpioa, 8), 7)),  // CH1  = PA8  AF7
        Some((Pin::new(p.gpioa, 7), 7)),  // CH1N = PA7  AF7
        Some((Pin::new(p.gpioa, 9), 7)),  // CH2  = PA9  AF7
        Some((Pin::new(p.gpiob, 0), 7)),  // CH2N = PB0  AF7
        Some((Pin::new(p.gpioa, 10), 7)), // CH3  = PA10 AF7
        Some((Pin::new(p.gpiob, 1), 7)),  // CH3N = PB1  AF7
        no_pin,                           // CH4 单端(本例不用)
        Some((Pin::new(p.gpioa, 6), 7)),  // BK   = PA6  AF7(低有效)
        false,                            // BKP=0:低电平有效
        0,                                // BKF=0:无数字滤波
        Irqs,
    );
    let mut pwm = match pwm {
        Ok(t) => t,
        Err(e) => {
            unsafe {
                ERR = match e {
                    AtimError::FreqOutOfRange => 1,
                    AtimError::DeadTimeTooLong => 2,
                    AtimError::BadBreakConfig => 3,
                    AtimError::BadDuty => 4,
                };
            }
            loop {
                core::hint::spin_loop();
            }
        }
    };

    let (psc, arr) = pwm.period_config();
    unsafe {
        PERIOD = (psc as u32) << 16 | arr as u32;
        DTG = pwm.dtg() as u32;
    }

    // 置 MOE:构造时 MOE=0(输出空闲全低)。BK 已活跃则置不上 → 返回 false
    if !pwm.enable() {
        unsafe { ENABLE_FAILS += 1 };
    }

    // 占空比 0→100%→0 三角斜坡,三路错 120°(万分比 3334/6667):
    // 非正弦,仅演示三相互补输出与 120° 相位观感
    let mut step: u32 = 0;
    loop {
        let d = ramp(step);
        pwm.set_duty_bp(1, d).ok();
        pwm.set_duty_bp(2, (d + 3_334) % 10_000).ok();
        pwm.set_duty_bp(3, (d + 6_667) % 10_000).ok();

        // 等一个真实 PWM 周期(ATIM 更新中断 + waker 异步路径)
        pwm.wait_update().await;
        unsafe { UPDATES += 1 };

        // 刹车监控:BIF 置位即六路输出已被硬件异步压到安全态(MOE=0)。
        // clear_break 在刹车源仍活跃时返回 false;源释放后重新 enable。
        if pwm.is_braked() {
            unsafe { BRAKES += 1 };
            if pwm.clear_break() && !pwm.enable() {
                unsafe { ENABLE_FAILS += 1 };
            }
        }
        step += 1;
    }
}
