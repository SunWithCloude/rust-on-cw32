# 第 13 章 RTC 与看门狗

这一章你会学到：

- RTC 的日历/闹钟寄存器模型与 LSE/LSI 时基选择（§13）；
- KEY 解锁序列（0xCA→0x53）为什么每次写都要走一遍、为什么必须进临界区；
- `wait_alarm().await` 的异步语义（睡到下一次匹配，而非等到固定时刻）；
- IWDT 与 WWDGT 的差别，以及 `WindowConfig::for_feed_interval` 在算什么。

## RTC：日历、闹钟与时基

RTC 是掉电域里的独立计时器（§13，寄存器映射表 13-6）：BCD 日历
（DATE/TIME，年月日星期时分秒）、两个闹钟（ALARMA/B）、亚秒计数
（SSCNT）。时基二选一（CR1.SOURCE，§13.5）：

- **LSE**：外部 32.768 kHz 晶体，最准；板上没焊晶体时起振等待（约 2 s）
  超时，`Rtc::new` 返回 `RtcError::ClockNotReady`；
- **LSI**：内部 32.768 kHz RC，出厂已 TRIM——本书作者板上实测 32768 Hz、
  误差 <1%（阻塞版 rtc.rs 注释）。

默认预分频 `PSC1=0 / PSC2=0x3FFF` 把 32768 Hz 分成 **2 Hz 的 TICKCLK**：
日历每半秒走一步。这也决定了闹钟粒度——闹钟按**秒字段的 BCD 匹配**
（时/分/秒任一字段可屏蔽），匹配判定每 0.5 s 一次，实际触发沿最多滞后
约 0.5 s。`Alarm` 用 `Option<u8>` 表示字段屏蔽：`second: Some(10)` 配
`hour: None, minute: None` 就是"每分钟的第 10 秒"。

## KEY 解锁：0xCA → 0x53 进临界区

RTC 除 `KEY`/`ICR` 外全部寄存器带写保护（手册表 13-6 注）：**每次写都要
先写 0xCA、再写 0x53 解锁，写完 0xCA→0xFF 重新上锁**。驱动把整个
"解锁 → 写 → 上锁"序列包进 `critical_section::with`——这不是花架子：
IRQ 处理器（`RtcIrq`）也要动 IER（压掉中断使能同样是 KEY 序列），如果
中断插进任务侧的解锁序列中间，对方的写会被静默顶掉。两代上下文共享一个
KEY 状态机，临界区是必要加固。

## wait_alarm 的异步语义

`Rtc::wait_alarm().await` **睡到闹钟 A 的下一次匹配**，不是"等到某个
绝对时刻"：返回时 ISR.ALARMA 已被 ICR（R1W0）清掉，再次调用就是等下一
次匹配，无需手动清标志。等待期间才开 IER.ALARMA（KEY 解锁写），IRQ2
唤醒后单发压掉。两个注意点：若调用时匹配标志已经在（配置的目标秒刚
过、或热复位残留），本次立即返回——介意就先 `clear_flags(IRQ_ALARMA)`；
IWDT 复位不清 RTC 域，热复位残留标志是真实存在的场景。

前置条件：RTC 中断绑定到 `rtc::RtcIrq`（`bind_interrupts!`），闹钟本身
先 `set_alarm_a` + `enable_alarm_a(true)` 配好。

## 看门狗：IWDT 与 WWDGT

两只看门狗（IWDT §19、WWDGT §20）都是**非异步薄封装**：喂狗是一次键值
写，没有可等待的事件；溢出"事件"本身就是要复位，异步化没有收益。选型
按场景：

| 场景 | 选择 | 依据 |
|---|---|---|
| 防主时钟跑飞 / 系统级兜底 | `Iwdt` | 计数时钟来自独立 RC10K（≈10 kHz），主时钟崩溃仍能复位 |
| 检测主循环时序异常 | `Wwdgt` | 窗口内才许喂狗，过早/过晚都复位 |
| DeepSleep 期间也要盯着 | `Iwdt` | WWDGT 在 DeepSleep 期间停计（§20），睡久会被误咬 |

**IWDT** 是键值机：`0xCCCC` 启动、`0xAAAA` 喂狗、`0x5555` 解锁改配置
（§19）。超时 `T = (PRS 分频) × (ARR+1) / RC10K`，范围 0.4 ms ~ 约
209.7 s（RC10K 有 ±% 级漂移，喂狗间隔留足裕量）——`wdt.start(TimeoutMs
(1500))` 内部自动选最小够用的分频组合。

**WWDGT** 是 7 位递减计数器（PCLK/4096/2^PRS 驱动），三个条件触发复位
（§20）：计到 0x3F（过晚）、当前计数值 > 窗口值 WINR 时执行喂狗（过早）、
向 WCNT 写入 ≤ 0x3F 的值。喂狗时间窗 `T_min = T_PCLK × 4096 × 2^PRS ×
(WCNT − WINR)`、`T_max = … × (WCNT − 0x3F)`（§20.3.4）。手算这套参数很
容易错，驱动提供求解器：`WindowConfig::for_feed_interval(pclk_hz,
interval_ms)` 按"每 interval_ms 喂一次"自动求 (PRS, WCNT, WINR)，让软件
喂狗点落在窗口正中、两侧各留约 25% 裕量。⚠ WWDGT 一旦开启**不可关闭**
（CR0.EN 为 RW1，仅系统复位可停）。

两条看门狗共享 IRQ0（溢出/预溢出中断用）——复位模式下不需要中断，驱动
未提供异步接口。

## 例程：RTC 异步闹钟

`examples/embassy/examples/ch13_rtc_alarm.rs`：零接线。LSE 失败退 LSI
（本板无 LSE 晶振，预期 `CLK_SRC=2`）；设 23:59:50 起跑；闹钟 A = 当前
秒 +5，唤醒后重设下一次——`WAKE_COUNT` 每 ~5 s +1；独立心跳任务
500 ms 翻转 PC13，**闹钟等待期间 LED 不停闪**，就是"等闹钟不阻塞执行
器"的直观证据。

```rust
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

use cw32_book_embassy::board;
use embassy_cw32::{
    Config, Peripherals,
    gpio::Output,
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
    // PC13 心跳灯：门控 + 构造在 board::led() 里（crate lib 的板级封装）
    let led = board::led();
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
```

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch13_rtc_alarm -p cw32-book-embassy
```

## 板上验证

**状态：仅编译未上板。** 本例程与 embassy-cw32 仓的 `rtc_alarm` 示例
同源；时基/日历/闹钟的寄存器配方与阻塞版 `rtc_demo` 逐行同源且已实机
验证（hal 仓 TESTING.md，2026-10-23：LSE 失败退 LSI、跨日走时 +
星期递增正确、闹钟命中时刻与设定一致）。未上板的增量是 embassy 的
`wait_alarm` 异步路径。

首次上板核对点：

- `CLK_SRC=2`（LSI 兜底生效）、`ERR=0`；
- `WAKE_COUNT` 每 ~5 s +1，且唤醒后读到的日历在正确走时；
- PC13 全程 1 Hz 心跳（`wait_alarm` 睡眠期间心跳不停）；
- 若 `WAKE_COUNT` 一开始就异常 +1：热复位残留了 ALARMA 标志（例程已
  在首次等待前 `clear_flags`，仍复现则检查复位源）。

看门狗侧：阻塞版 `wdt_demo`/`wwdt_demo` 均已实机验证（IWDT 停喂 ~1.5 s
后复位循环、WWDT 50 轮过早喂狗全部被拦截），embassy 仓的 `wdt_demo`
（IWDT 复位循环）属仅编译未上板——本书正文结论引用阻塞版实测记录。

## 常见坑

- **停表状态下 CR1.WAIT 不会清零**：手册 §13.3.6 的"写 DATE/TIME 前等
  WAIT"只适用于**运行中**写；`set_datetime` 驱动内已按此处理（先停表
  再写），自己摸寄存器时照手册流程会误报超时（阻塞版实测教训）。
- **KEY 序列被打断 = 写静默丢失**：不要在任务里裸写 KEY 再写寄存器，
  中断一插进来写就没了——用驱动封装，序列已进临界区。
- **读日历跨秒**：连读 DATE/TIME 可能撞上进位边界，`rtc.now()` 连读两次
  取稳定值（§13.3.6 推荐做法），自己读寄存器时别省这一步。
- **IWDT 一旦启动不可停**（0x5A5A+0xA5A5 双键仅供调试期使用，产线代码
  不应出现）；WWDGT 更干脆，开了就关不掉。
- **喂狗间隔贴着超时下限**：RC10K 有 ±% 级漂移，"算出来刚好"的喂狗周期
  在温漂下会偶发复位——留 2 倍以上裕量，或用 WWDGT 的窗口求解器让喂狗
  点落在窗口正中。
