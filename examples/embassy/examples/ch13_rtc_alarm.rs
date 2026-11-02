//! 第 13 章例程：RTC 异步闹钟（零接线）。
//!
//! 1. 时基：优先 LSE（外部 32.768kHz 晶体，起振等待 ~2s），失败退 LSI
//!    （内部 RC，出厂 TRIM；本板无 LSE 晶振，预期 CLK_SRC=2）
//! 2. 设 2026-10-31（周六）23:59:50 并启动计时
//! 3. 闹钟 A = 当前秒 +5（时/分屏蔽 → 每分钟该秒触发），`wait_alarm().await`
//!    睡到触发后重设下一次——WAKE_COUNT 每 ~5s +1
//! 4. 心跳：独立任务 500ms 翻转 PC13——闹钟等待期间 LED 不停闪，
//!    即"等闹钟不阻塞执行器"的直观证据
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch13_rtc_alarm -p cw32-book-embassy
//! 状态：仅编译未上板（embassy 版 rtc_alarm 同状态）；时基/日历/闹钟配方
//!       与阻塞版 rtc_demo 同源且已实机验证（跨日走时 + 闹钟命中，
//!       2026-10-23，见 hal TESTING.md）。首次上板核对点见第 13 章正文。

#![no_std]
#![no_main]

use embassy_executor::{Spawner, main};
use embassy_time::Timer;

use embassy_cw32::{
    Config, Peripherals,
    gpio::{Level, Output, Pin},
    rtc::{Alarm, DateTime, Rtc, RtcClock},
};

embassy_cw32::bind_interrupts!(struct Irqs {
    RTC => embassy_cw32::rtc::RtcIrq;
});

/// 实际使用的时钟源：1=LSE、2=LSI、0=双失败（挂死待查）
#[used]
#[unsafe(no_mangle)]
pub static mut CLK_SRC: u32 = 0;
/// 闹钟唤醒计数（每 ~5s +1）
#[used]
#[unsafe(no_mangle)]
pub static mut WAKE_COUNT: u32 = 0;
/// 错误码：0=无 1=BadDateTime 2=ClockNotReady
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;

/// PC13 心跳：独立任务 500ms 翻转，闹钟等待期间持续闪烁
#[embassy_executor::task]
async fn heartbeat(mut led: Output<embassy_cw32::Gpio>) {
    loop {
        led.toggle();
        Timer::after_millis(500).await;
    }
}

#[main]
async fn main(spawner: Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk = 4MHz

    let p = Peripherals::take().unwrap();
    embassy_cw32::sysctrl_enable_gpioc(p.sysctrl);
    let led = Output::new(Pin::new(p.gpioc, 13), Level::High);
    // embassy-executor 0.10：spawn() 返回 ()（SpawnToken 构造失败时内部 panic）
    spawner.spawn(heartbeat(led).unwrap());

    // ---- 时基：LSE 优先，失败退 LSI（阻塞版 rtc_demo 实机配方）----
    let mut rtc = match Rtc::new(p.rtc, RtcClock::Lse) {
        Ok(r) => {
            unsafe { CLK_SRC = 1 };
            r
        }
        Err(_) => match Rtc::new(p.rtc, RtcClock::Lsi) {
            Ok(r) => {
                unsafe { CLK_SRC = 2 };
                r
            }
            Err(_) => {
                unsafe { ERR = 2 }; // ClockNotReady：双时钟源都失败
                loop {
                    Timer::after_secs(1).await; // 心跳仍在跑，便于观测挂死态
                }
            }
        },
    };

    // ---- 设 2026-10-31（周六）23:59:50，启动计时 ----
    let t0 = DateTime {
        year: 26,
        month: 10,
        day: 31,
        weekday: 6,
        hour: 23,
        minute: 59,
        second: 50,
    };
    if rtc.set_datetime(&t0).is_err() {
        unsafe { ERR = 1 }; // BadDateTime
    }
    rtc.start();

    // ---- 闹钟 A = 当前秒 +5（分/时屏蔽 → 每分钟该秒触发）----
    let target = (rtc.now().second + 5) % 60;
    rtc.set_alarm_a(&Alarm {
        weekmask: 0x7F, // 每天都响
        hour: None,
        minute: None,
        second: Some(target),
    });
    rtc.enable_alarm_a(true);
    // 清热复位残留标志（IWDT 复位不清 RTC 域）：保证首次 wait 睡到
    // 下一次真实匹配而不是立即返回
    rtc.clear_flags(embassy_cw32::rtc::IRQ_ALARMA);

    loop {
        rtc.wait_alarm().await; // ← 异步睡到闹钟（IRQ2 唤醒，心跳不受影响）
        unsafe { WAKE_COUNT += 1 };
        // 下一个闹钟：当前秒 +5（闹钟按秒字段 BCD 匹配，粒度见正文）
        let target = (rtc.now().second + 5) % 60;
        rtc.set_alarm_a(&Alarm {
            weekmask: 0x7F,
            hour: None,
            minute: None,
            second: Some(target),
        });
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
