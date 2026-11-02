//! 板级小封装：PC13 板载 LED 一步构造。
//!
//! 只收敛每个异步例程都要重复的两行（门控 + 构造输出引脚）；涉及
//! GPIOA/GPIOB 等多端口的例程请自行写 AHBEN 组合字（键控整字写，
//! 见第 7/8 章），**不要**与 [`led`] 的门控混用。

use embassy_cw32::{
    Peripherals,
    gpio::{Level, Output, Pin},
};

/// PC13 板载 LED：开 GPIOC 门控（`sysctrl_enable_gpioc`）→ 退模拟态、
/// 方向输出、置初始电平（`Output::new`），等价于例程里的
///
/// ```ignore
/// embassy_cw32::sysctrl_enable_gpioc(p.sysctrl);
/// let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);
/// ```
///
/// 外设单例经 `Peripherals::steal()` 取得（寄存器块都在固定地址，
/// steal 只是解引用，不重复配置任何外设），因此不影响例程自己
/// `Peripherals::take()` 其余外设。
pub fn led() -> Output<embassy_cw32::Gpio> {
    // SAFETY: 仅借地址取 &'static 寄存器块句柄，无配置副作用；单核
    // 裸机下与 take() 不会产生并发冲突。
    let p = unsafe { Peripherals::steal() };
    embassy_cw32::sysctrl_enable_gpioc(p.sysctrl);
    Output::new(Pin::new(p.gpioc, 13), Level::High)
}
