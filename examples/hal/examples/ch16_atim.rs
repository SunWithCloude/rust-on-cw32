//! 第 16 章例程:ATIM 互补 PWM + 死区 + 刹车(PA08=CH1、PA15=CH1N、PA06=BK)。
//!
//! 与 cw32l012-hal examples/atim_deadtime.rs 同源(仅静态量加
//! `#[used]`+`#[unsafe(no_mangle)]` 便于 probe-rs 直接读符号,并按
//! edition 2024 调整属性写法)。2kHz、30% 占空比、死区 250µs(4MHz
//! PCLK 下向上取整到 DTG=0xFF,即 1008 t_DTS ≈ 252µs)。
//!
//! 验证方式(无示波器也可,接调试器读静态量):
//! - `REG` 回读 (ARR<<16 | PSC)——预期 0x07CF_0000(ARR=1999、PSC=0)
//! - `CCR` 回读——预期 600(2000×30%)
//! - `BDTR` 回读——预期 0x94FF(MOE=1、OSSI=1、BKE=1、DTG=0xFF)
//! - `CNT_MIN/MAX`——预期铺满 0..1999
//! - 接示波器:PA08/PA15 互补方波,翻转沿间隔 ≈252µs 互不重叠;
//!   拉低 PA06(ATIM_BK)两路同时熄灭(MOE 被硬件清零,`BDTR` 读回
//!   0x14FF),`re_enable_output()` 恢复。
//!
//! 构建:cargo build --release --target thumbv6m-none-eabi --example ch16_atim -p cw32-book-hal
//! 状态:同源已验证——对应 hal 仓 TESTING.md 实测通过表 atim_deadtime 条目
//!       (REG=0x07CF0000、CCR=600、BDTR=0x94FF、CNT 铺满、PA06 刹车熄灭);
//!       本组合文件本身仅编译未上板。

#![no_std]
#![no_main]

use cw32_book_hal as _; // 继承 lib.rs 的 panic handler(panic-probe 同款机制)
use cw32l012_hal::{
    atim::{Atim, AtimChannel},
    gpio::{Af, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

/// (ARR<<16 | PSC) 配置回读:2kHz@PCLK 4MHz 预期 0x07CF_0000
#[used]
#[unsafe(no_mangle)]
pub static mut REG: u32 = 0;
/// 最近一次 CCR 回读:30% 占空比预期 600
#[used]
#[unsafe(no_mangle)]
pub static mut CCR: u16 = 0;
/// BDTR 回读(MOE/DTG/刹车位):预期 0x94FF
#[used]
#[unsafe(no_mangle)]
pub static mut BDTR: u32 = 0;
/// CNT 采样最小值:预期 0(计数器真的在滚)
#[used]
#[unsafe(no_mangle)]
pub static mut CNT_MIN: u16 = 0xFFFF;
/// CNT 采样最大值:预期 1999
#[used]
#[unsafe(no_mangle)]
pub static mut CNT_MAX: u16 = 0;

#[unsafe(no_mangle)]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };

    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks(); // PCLK = 4MHz

    // 互补输出引脚:PA08 = ATIM_CH1、PA15 = ATIM_CH1N(均 AF7,手册表 9-2)
    let _ch1 = Pin::new(p.gpioa, PortId::A, 8, &mut rcc).into_af_output(Af::Af7);
    let _ch1n = Pin::new(p.gpioa, PortId::A, 15, &mut rcc).into_af_output(Af::Af7);
    // 刹车输入 PA06 = ATIM_BK:必须走 AF7 复用输入(内部有滤波/极性电路,
    // 普通 GPIO 输入模式采样不到 BRK);外部上拉保持高电平 = 不刹车,
    // 拉低 = 刹车。悬空会让 BRK 随噪声误触发(实机抓到的坑:普通输入
    // 模式下 MOE 被清,输出无波形,BDTR 读回 0x14FF)
    let _bk = Pin::new(p.gpioa, PortId::A, 6, &mut rcc).into_af_input(Af::Af7);

    let mut tim = Atim::new(p.atim, &mut rcc);
    // 2kHz @ PCLK 4MHz:PSC=0、ARR=1999;30% 占空比;死区 250µs
    // (encode_dtg 向上取整 → DTG=0xFF,1008 t_DTS ≈ 252µs)
    if tim
        .start_pwm_complementary(AtimChannel::Ch1, clocks.pclk_hz, 2_000, 3_000, 250_000)
        .is_err()
    {
        loop {
            core::hint::spin_loop();
        }
    }
    // 刹车:BRK 低电平有效(外部拉低 → MOE 硬件清零 → 输出安全态)
    tim.enable_brake(false, 0);
    tim.re_enable_output(); // 若上电瞬间有毛刺已锁存刹车,此处恢复主输出

    unsafe {
        let (arr, psc) = tim.period_config();
        REG = (arr as u32) << 16 | psc as u32;
        CCR = tim.ccr(AtimChannel::Ch1);
        BDTR = tim.bdtr();
    }

    // 运行观测:CNT 滚动范围(证明计数器活着);需要时也可在此周期性
    // set_duty 做呼吸——占空比下一周期才生效(OCxPE 预装载)
    loop {
        unsafe {
            let c = tim.count();
            if c < CNT_MIN {
                CNT_MIN = c;
            }
            if c > CNT_MAX {
                CNT_MAX = c;
            }
        }
    }
}
