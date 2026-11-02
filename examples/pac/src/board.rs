//! 板级小封装：SysTick 忙等延时 + PC13 板载 LED。
//!
//! 抽进 lib 的动机见第 2 章"工程化"一节：延时与点灯是每个例程都要
//! 重复的样板。寄存器路径与 `examples/ch02_blink.rs` 逐行同源：
//! 门控（KEY 写保护）→ 退模拟态 → 方向输出 → TOG/BSRR 翻转。

use cortex_m::peripheral::syst::SystClkSource;
use cortex_m::peripheral::Peripherals as CorePeripherals;
use cw32l012_pac::{Peripherals, sysctrl as sc, Gpio};

/// PC13 在端口寄存器中的位掩码（GPIO 位段均按 PINy 编号：PIN13 = bit13）
pub const PIN13: u32 = 1 << 13;

/// 复位默认主频：HCLK = 4 MHz（HSI，手册 §4.3.4 / §4.7.4）。
/// 若固件改了主频，请同步修改这个常量——延时长短会按比例失准。
pub const HCLK_HZ: u32 = 4_000_000;

// ================= SysTick 延时 =================

// SysTick 是 ARMv6-M 的**核心**外设（寄存器组在系统控制空间
// 0xE000_E010，ARMv6-M 架构参考手册 DDI 0419 §B3.3），不属于 CW32L012
// 的芯片外设，因此手写 PAC（cw32l012-pac）没有建模它——延时只能经由
// cortex-m crate 的 `SYST` 类型访问（该 crate 已在工作区依赖树中，
// 经 embassy-executor 的 cortex-m 平台层引入；examples/pac 显式声明
// 0.7 以对齐版本）。
//
// 原理（三件事）：`RVR`（LOAD）装"数多少拍"；`CVR`（VAL）写任意值即
// 清零重启；`CSR`（CTRL）的 CLKSOURCE=1 选 HCLK、ENABLE=1 启动计数、
// **COUNTFLAG 在数到 0 时置位、读 CSR 即自动清零**——轮询它就得到了
// "一次定时"。本模块每个单位毫秒/分段微秒发一发单次倒数。

/// 每微秒的 SysTick 节拍数（CLKSOURCE=1：直接数 HCLK）
const TICKS_PER_US: u32 = HCLK_HZ / 1_000_000;
/// 24 位计数器单次倒数最多多少微秒（2^24 / TICKS_PER_US ≈ 4.19 s）
const MAX_US: u32 = (1 << 24) / TICKS_PER_US;

/// 微秒级忙等延时（SysTick 单次倒数 + COUNTFLAG 轮询）。
/// 超过单次量程（约 4.19 s @4 MHz）自动分段续数。
pub fn delay_us(us: u32) {
    let mut left = us;
    while left > 0 {
        let chunk = if left > MAX_US { MAX_US } else { left };
        one_shot(chunk * TICKS_PER_US);
        left -= chunk;
    }
}

/// 毫秒级忙等延时：逐毫秒一发单次倒数。每发之间重装初值的固定开销
/// 只有几十个周期，远小于 1 ms，对"肉眼可见的节奏"完全无感。
pub fn delay_ms(ms: u32) {
    for _ in 0..ms {
        delay_us(1000);
    }
}

/// 装载 `ticks` 个节拍，数到 0 才返回（阻塞）。
fn one_shot(ticks: u32) {
    // SAFETY: 仅借核心外设句柄读写 SysTick 三个寄存器，不触碰任何
    // 芯片外设配置；单核裸机、调用发生在使能中断之前，无竞争。
    let mut cp = unsafe { CorePeripherals::steal() };
    let syst = &mut cp.SYST;
    syst.set_clock_source(SystClkSource::Core); // CLKSOURCE=1：数 HCLK
    syst.set_reload(ticks - 1); // RVR：从 ticks-1 倒数到 0
    syst.clear_current(); // CVR 写任意值：计数清零（COUNTFLAG 一并清掉）
    syst.enable_counter(); // ENABLE=1
    while !syst.has_wrapped() {
        core::hint::spin_loop(); // 数到 0 置 COUNTFLAG，读 CSR 即自动清零
    }
    syst.disable_counter(); // 下一次调用重新编程，平时让计数器停着
}

// ================= PC13 LED =================

/// PC13 板载 LED 封装。
///
/// 构造即完成 ch02 的三步寄存器序列；翻转/置平走 TOG/BSRR 只写寄存器。
/// 门控一步采用"读低 16 位现状 + 补 KEY 写回"的读改写（与
/// cw32l012-hal `rcc::set_bits` 同序），**保留**同一固件里已打开的
/// 其它端口门控位。
pub struct Led {
    gpioc: &'static Gpio,
}

impl Led {
    /// 从已取出的外设单例构造 PC13 LED。
    ///
    /// 内部三步与 `ch02_blink.rs` 第 1~3 步逐行同源：GPIOC 门控
    /// （AHBEN，KEY 写保护，手册 §4.7.11）→ ANALOG 清零（§9.6.7）→
    /// DIR 输出（§9.6.1，L012 的 DIR=0 才是输出）。
    pub fn new(p: &Peripherals) -> Self {
        // KEY 位读出恒为 0：改位前先读低 16 位现状，写回时原样补 KEY，
        // 否则整次写入被硬件静默丢弃
        let v = p.sysctrl.ahben.read() & 0xFFFF;
        p.sysctrl.ahben.write(sc::KEY | v | sc::AHBEN_GPIOC);
        Self::from_gpioc(p.gpioc)
    }

    /// AHBEN 已自行配好时（例如与其它端口位拼在同一次写里）的构造入口：
    /// 只做退模拟态与方向两步。
    pub fn from_gpioc(gpioc: &'static Gpio) -> Self {
        // 复位默认 GPIOx_ANALOG = 0xE000（GPIOC）：PIN13=1 模拟态，
        // 数字输入输出被断开，必须清零（手册 §9.6.7 / §9.4.1 步骤 1）
        gpioc.analog.modify(|v| v & !PIN13);
        // DIR.PIN13 清零 = 输出（§9.6.1：与多数 MCU 相反，1 是输入）
        gpioc.dir.modify(|v| v & !PIN13);
        Led { gpioc }
    }

    /// 翻转电平：写 TOG 位掩码，硬件原子翻转 ODR（手册 §9.6.17）。
    pub fn toggle(&mut self) {
        self.gpioc.tog.write(PIN13);
    }

    /// 置平：true = 输出高电平（BSRR 置位段），false = 输出低电平
    /// （BSRR 清零段，即位掩码左移 16）。
    ///
    /// 注意：**"亮"对应哪个电平取决于板卡 LED 的接法**（拉电流还是灌
    /// 电流），本封装不承诺——第 2 章例程一律用 `toggle()` 通吃两种接法。
    pub fn set(&mut self, high: bool) {
        let bits = if high { PIN13 } else { PIN13 << 16 };
        self.gpioc.bsrr.write(bits);
    }

    /// 输出高电平（是否点亮取决于板卡接法，见 [`Led::set`]）
    pub fn on(&mut self) {
        self.set(true);
    }

    /// 输出低电平（是否点亮取决于板卡接法，见 [`Led::set`]）
    pub fn off(&mut self) {
        self.set(false);
    }
}
