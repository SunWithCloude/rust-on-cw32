# Chapter 13 RTC and Watchdogs

In this chapter you will learn:

- The RTC's calendar/alarm register model and the LSE/LSI time-base choice
  (§13);
- Why the KEY unlock sequence (0xCA→0x53) has to run on every write, and
  why it must happen inside a critical section;
- The async semantics of `wait_alarm().await` (sleep until the next match,
  not until a fixed moment);
- The difference between IWDT and WWDGT, and what
  `WindowConfig::for_feed_interval` is computing.

## RTC: Calendar, Alarms, and the Time Base

The RTC is an independent timer in the power-down domain (§13, register map
Table 13-6): a BCD calendar (DATE/TIME, year/month/day/weekday +
hours/minutes/seconds), two alarms (ALARMA/B), and a subsecond counter
(SSCNT). The time base is one of two (CR1.SOURCE, §13.5):

- **LSE**: an external 32.768 kHz crystal, the most accurate; when no
  crystal is soldered on the board, the start-up wait (about 2 s) times out
  and `Rtc::new` returns `RtcError::ClockNotReady`;
- **LSI**: an internal 32.768 kHz RC, factory-TRIMmed — measured on the
  author's board at 32768 Hz, error <1% (blocking rtc.rs comment).

The default prescalers `PSC1=0 / PSC2=0x3FFF` divide 32768 Hz into a
**2 Hz TICKCLK**: the calendar advances one step every half second. This
also sets the alarm granularity — alarms match on the **BCD fields of the
clock** (any of the hour/minute/second fields can be masked), the match is
evaluated every 0.5 s, and the actual trigger edge can lag by up to about
0.5 s. `Alarm` uses `Option<u8>` for field masks: `second: Some(10)` with
`hour: None, minute: None` means "the 10th second of every minute".

## The KEY Unlock: 0xCA → 0x53 Inside a Critical Section

Every RTC register except `KEY`/`ICR` is write-protected (manual Table 13-6
note): **every write must first write 0xCA, then 0x53, to unlock; when the
write is done, 0xCA→0xFF re-locks**. The driver wraps the whole
"unlock → write → re-lock" sequence in `critical_section::with` — this is
not decoration: the IRQ handler (`RtcIrq`) also touches IER (dropping the
interrupt enable is itself a KEY sequence), and if an interrupt lands in
the middle of the task-side unlock sequence, the other side's write is
silently clobbered. Two generations of context share one KEY state machine,
so the critical section is a necessary hardening.

## The Async Semantics of wait_alarm

`Rtc::wait_alarm().await` **sleeps until alarm A's next match**, not "until
some absolute moment": on return ISR.ALARMA has already been cleared by the
ICR (R1W0), so calling it again waits for the next match, with no manual
flag clearing. IER.ALARMA is enabled (a KEY-unlocked write) only while
waiting, and dropped after the IRQ2 wake, once. Two caveats: if the match
flag is already set at call time (the configured target second just passed,
or a leftover from a warm reset), this call returns immediately — if that
bothers you, call `clear_flags(IRQ_ALARMA)` first; an IWDT reset does not
clear the RTC domain, so warm-reset leftovers are a real scenario.

Preconditions: the RTC interrupt is bound to `rtc::RtcIrq`
(`bind_interrupts!`), and the alarm itself is configured beforehand with
`set_alarm_a` + `enable_alarm_a(true)`.

## Watchdogs: IWDT and WWDGT

Both watchdogs (IWDT §19, WWDGT §20) are **non-async thin wrappers**:
feeding the watchdog is a single key-value write with nothing to wait for,
and the overflow "event" is by definition a reset — making it async brings
no benefit. Pick by scenario:

| Scenario | Choice | Rationale |
|---|---|---|
| Guarding against a runaway main clock / system-level backstop | `Iwdt` | The counting clock comes from the independent RC10K (≈10 kHz); it still resets when the main clock collapses |
| Detecting main-loop timing anomalies | `Wwdgt` | Feeding is allowed only inside the window; too early or too late both reset |
| Must keep watch through DeepSleep | `Iwdt` | WWDGT stops counting during DeepSleep (§20); a long sleep gets bitten by mistake |

**IWDT** is a key-value machine: `0xCCCC` to start, `0xAAAA` to feed the
watchdog, `0x5555` to unlock the configuration (§19). The timeout is
`T = (PRS division) × (ARR+1) / RC10K`, spanning 0.4 ms to about 209.7 s
(RC10K drifts at the ±% level, so leave ample margin in the feed interval) —
`wdt.start(TimeoutMs(1500))` automatically picks the smallest sufficient
divider combination.

**WWDGT** is a 7-bit down-counter (driven by PCLK/4096/2^PRS) with three
reset conditions (§20): counting down to 0x3F (too late), feeding while the
current count > the window value WINR (too early), and writing a value
≤ 0x3F to WCNT. The feed window is `T_min = T_PCLK × 4096 × 2^PRS ×
(WCNT − WINR)` and `T_max = … × (WCNT − 0x3F)` (§20.3.4). Hand-computing
these parameters is error-prone, so the driver provides a solver:
`WindowConfig::for_feed_interval(pclk_hz, interval_ms)` solves
(PRS, WCNT, WINR) automatically for "feed once per interval_ms", landing
the software feed point in the middle of the window with about 25% margin
on either side. ⚠ Once enabled, WWDGT **cannot be turned off** (CR0.EN is
RW1; only a system reset stops it).

The two watchdogs share IRQ0 (for the overflow/pre-overflow interrupts) —
not needed in reset mode, and the driver provides no async interface.

## The Example: Async RTC Alarm

`examples/embassy/examples/ch13_rtc_alarm.rs`: zero wiring. LSE failure
falls back to LSI (this board has no LSE crystal, expect `CLK_SRC=2`);
starts at 23:59:50; alarm A = current second +5, and on each wake the next
one is re-armed — `WAKE_COUNT` +1 every ~5 s; an independent heartbeat task
toggles PC13 every 500 ms, and **the LED keeps blinking while the alarm
wait is in progress** — direct evidence that "waiting for the alarm does
not block the executor".

```rust
//! Chapter 13 example: async RTC alarm (zero wiring).
//!
//! 1. Time base: LSE first (external 32.768kHz crystal, ~2s start-up wait),
//!    fall back to LSI on failure (internal RC, factory TRIM; no LSE
//!    crystal on this board, expect CLK_SRC=2)
//! 2. Set 2026-10-31 (Saturday) 23:59:50 and start the clock
//! 3. Alarm A = current second +5 (hour/minute masked → fires at that
//!    second every minute); `wait_alarm().await` sleeps until it fires and
//!    then re-arms the next one — WAKE_COUNT +1 every ~5s
//! 4. Heartbeat: an independent task toggles PC13 every 500ms — the LED
//!    keeps blinking while the alarm wait is in progress, direct evidence
//!    that "waiting for the alarm does not block the executor"
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch13_rtc_alarm -p cw32-book-embassy
//! Status: compiles, not yet run on hardware (same state as the embassy
//!       version's rtc_alarm); the time-base/calendar/alarm recipe is
//!       same-source as the blocking rtc_demo and verified on hardware
//!       (day rollover + alarm hit, 2026-10-23, see the hal repo's
//!       TESTING.md). First-flash checklist in the Chapter 13 text.

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

/// Clock source actually in use: 1=LSE, 2=LSI, 0=both failed (hung, under investigation)
#[used]
#[unsafe(no_mangle)]
pub static mut CLK_SRC: u32 = 0;
/// Alarm wake counter (+1 every ~5s)
#[used]
#[unsafe(no_mangle)]
pub static mut WAKE_COUNT: u32 = 0;
/// Error code: 0=none 1=BadDateTime 2=ClockNotReady
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;

/// PC13 heartbeat: an independent task toggles every 500ms, blinking on through the alarm wait
#[embassy_executor::task]
async fn heartbeat(mut led: Output<embassy_cw32::Gpio>) {
    loop {
        led.toggle();
        Timer::after_millis(500).await;
    }
}

#[main]
async fn main(spawner: Spawner) {
    embassy_cw32::init(Config::default()); // reset defaults: SysClk = 4MHz

    let p = Peripherals::take().unwrap();
    // PC13 heartbeat LED: gating + construction inside board::led() (the crate lib's board wrapper)
    let led = board::led();
    // embassy-executor 0.10: spawn() returns () (panics internally if SpawnToken construction fails)
    spawner.spawn(heartbeat(led).unwrap());

    // ---- Time base: LSE first, fall back to LSI (the blocking rtc_demo's hardware-proven recipe) ----
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
                unsafe { ERR = 2 }; // ClockNotReady: both clock sources failed
                loop {
                    Timer::after_secs(1).await; // the heartbeat is still running, making the hung state observable
                }
            }
        },
    };

    // ---- Set 2026-10-31 (Saturday) 23:59:50, start the clock ----
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

    // ---- Alarm A = current second +5 (minute/hour masked → fires at that second every minute) ----
    let target = (rtc.now().second + 5) % 60;
    rtc.set_alarm_a(&Alarm {
        weekmask: 0x7F, // fires every day
        hour: None,
        minute: None,
        second: Some(target),
    });
    rtc.enable_alarm_a(true);
    // Clear warm-reset leftover flags (an IWDT reset does not clear the RTC domain): makes the
    // first wait sleep until the next real match instead of returning immediately
    rtc.clear_flags(embassy_cw32::rtc::IRQ_ALARMA);

    loop {
        rtc.wait_alarm().await; // ← async sleep until the alarm (woken by IRQ2; the heartbeat is unaffected)
        unsafe { WAKE_COUNT += 1 };
        // Next alarm: current second +5 (alarms match BCD on the seconds field; granularity in the text)
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

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch13_rtc_alarm -p cw32-book-embassy
```

## On-Hardware Verification

**Status: compiles, not yet run on hardware.** This example is same-source
as the `rtc_alarm` example in the embassy-cw32 repo; the
time-base/calendar/alarm register recipe is line-for-line same-source as
the blocking `rtc_demo` and verified on hardware (hal repo TESTING.md,
2026-10-23: LSE failure falls back to LSI, timekeeping across midnight +
weekday increment correct, alarm hit exactly at the set moment). The
unverified increment is embassy's `wait_alarm` async path.

First-flash checklist:

- `CLK_SRC=2` (the LSI fallback took effect), `ERR=0`;
- `WAKE_COUNT` +1 every ~5 s, and the calendar read after each wake keeps
  correct time;
- PC13 1 Hz heartbeat throughout (the heartbeat does not stop while
  `wait_alarm` sleeps);
- If `WAKE_COUNT` bumps immediately at startup: an ALARMA flag was left
  over from a warm reset (the example already calls `clear_flags` before
  the first wait; if it still reproduces, check the reset source).

Watchdog side: the blocking `wdt_demo`/`wwdt_demo` are both verified on
hardware (IWDT reset loop after ~1.5 s of stopped feeding; all 50 WWDT
early feeds intercepted); the embassy repo's `wdt_demo` (IWDT reset loop)
is compiles-only, not yet run on hardware — the main text's conclusions
cite the blocking version's measured records.

## Common Pitfalls

- **CR1.WAIT never clears in the stopped state**: the manual §13.3.6 "wait
  for WAIT before writing DATE/TIME" applies only to writes **while
  running**; the `set_datetime` driver already handles this internally
  (stop the clock, then write). Following the manual's flow when
  hand-writing registers will misreport a timeout (a blocking-version
  lesson measured the hard way).
- **A broken KEY sequence = writes silently lost**: do not hand-write KEY
  and then registers inside a task — one interrupt in between and the write
  is gone; use the driver wrappers, whose sequence is already inside a
  critical section.
- **Reading the calendar across a second boundary**: reading DATE/TIME back
  to back can hit a carry boundary; read `rtc.now()` twice and take the
  stable value (the §13.3.6 recommended practice). Do not skip this step
  when reading the registers yourself.
- **Once started, IWDT cannot be stopped** (the 0x5A5A+0xA5A5 double key
  exists for debugging only and should never appear in production code);
  WWDGT is blunter still — once on, never off.
- **Feed intervals hugging the timeout floor**: RC10K drifts at the ±%
  level, and a feed period that "computes out exactly right" will
  occasionally miss under temperature drift — leave 2× or more margin, or
  use the WWDGT window solver to land the feed point mid-window.
