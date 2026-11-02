//! 第 6 章例程（下）：RTC 跨日走时 + IWDT 看门狗复位循环（零接线）。
//!
//! 段 A（RTC）：优先 LSE，不起振则退回 LSI（实测本板 LSE 未起振——
//! 没焊 32.768k 晶体或负载电容不匹配，实跑走 LSI）；设 2026-10-23（周五）
//! 23:59:50，启动后等 12s 读回——应已跨到次日（周六），一次验证 BCD
//! 进位、跨日与星期递增——与 cw32l012-hal examples/rtc_demo.rs 的 P1 同源。
//! 段 B（IWDT）：1.5s 超时；前 5 次喂狗（PC13 每次翻转 → 快闪 5 下），
//! 之后故意停喂 → 1.5s 后看门狗复位、固件从头再跑——与
//! examples/wdt_demo.rs 同源。
//!
//! 上电节奏：前 ~12s 灯不亮（RTC 在做跨日测试）→ 快闪 5 下 →
//! 灭 ~1.5s → 复位、再来一轮。走时结果在 CLK/DT_AFTER/TM_AFTER
//! 静态量（调试器可读）。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch06_rtc_iwdt -p cw32-book-hal
//! 状态：同源已验证——两段分别对应 TESTING.md 实测通过表的 rtc_demo
//!       （CLK=2 走 LSI、跨日 00:00:01、星期递增）与 wdt_demo（BOOT_ID
//!       增至 6 → 复位回 1 循环）条目；本组合文件本身仅编译未上板。

#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, write_volatile};

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    rtc::{DateTime, Rtc, RtcClock},
    wdt::{Iwdt, TimeoutMs},
    Peripherals,
};

/// 实际使用的 RTC 时钟源（1=LSE、2=LSI、0=失败）
static mut CLK: u32 = 0;
/// 12s 后读回的日期（year<<24 | month<<16 | day<<8 | weekday）
static mut DT_AFTER: u32 = 0;
/// 12s 后读回的时间（hour<<16 | minute<<8 | second）
static mut TM_AFTER: u32 = 0;
/// 本次开机喂狗次数（看门狗复位后重新从 0 爬升）
static mut BOOT_ID: u32 = 0;

fn pack_date(d: &DateTime) -> u32 {
    ((d.year as u32) << 24) | ((d.month as u32) << 16) | ((d.day as u32) << 8) | d.weekday as u32
}

fn pack_time(d: &DateTime) -> u32 {
    ((d.hour as u32) << 16) | ((d.minute as u32) << 8) | d.second as u32
}

#[unsafe(no_mangle)]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };

    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let mut delay = Delay::new(&rcc.clocks());
    let mut led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);

    // ---- 段 A：RTC 跨日走时（LSE 优先，失败退 LSI）----
    let rtc = match Rtc::new(p.rtc, &mut rcc, RtcClock::Lse) {
        Ok(r) => {
            unsafe { write_volatile(addr_of_mut!(CLK), 1) };
            r
        }
        Err(_) => match Rtc::new(p.rtc, &mut rcc, RtcClock::Lsi) {
            Ok(r) => {
                unsafe { write_volatile(addr_of_mut!(CLK), 2) };
                r
            }
            Err(_) => loop {
                core::hint::spin_loop();
            },
        },
    };

    // 设 23:59:50 → 启动 → 12s 后应已跨到次日（实测 00:00:01，周六；
    // 名义 :02，差 1s 为 LSI 走时与延时基准的合计偏差，见 TESTING.md）
    let t0 = DateTime {
        year: 26,
        month: 10,
        day: 23,
        weekday: 5, // 0=周日，5=周五
        hour: 23,
        minute: 59,
        second: 50,
    };
    if rtc.set_datetime(&t0).is_err() {
        // 只有字段非法或 RTC 同步异常才会到这里，正常上电不会发生；
        // 冻结在此便于发现问题（LED 保持不动也是信号）
        loop {
            core::hint::spin_loop();
        }
    }
    rtc.start();
    delay.delay_ms(12_000);
    let t1 = rtc.datetime();
    unsafe {
        write_volatile(addr_of_mut!(DT_AFTER), pack_date(&t1));
        write_volatile(addr_of_mut!(TM_AFTER), pack_time(&t1));
    }

    // ---- 段 B：IWDT 看门狗（1.5s 超时；喂 5 次停喂 → 复位循环）----
    let mut wdt = Iwdt::new(p.iwdt, &mut rcc);
    match wdt.start(TimeoutMs(1500)) {
        Ok(_) => {}
        Err(_) => loop {
            core::hint::spin_loop();
        },
    }

    loop {
        unsafe {
            BOOT_ID += 1;
            // 只喂前 5 次；之后故意停喂 → 1.5s 后看门狗复位
            if BOOT_ID <= 5 {
                led.toggle();
                wdt.feed();
            }
        }
        delay.delay_ms(600); // 喂狗间隔 600ms < 1.5s
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
