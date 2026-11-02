# Chapter 6: A Quick Tour of the Blocking Side: Timers, DMA, RTC, and the Watchdog

The peripherals of the previous two chapters (GPIO, UART) were all "the CPU
giving orders". This chapter changes the cast: a **timer** counts the beats
by itself, **DMA** moves data on the CPU's behalf, the **RTC** keeps time
even when the main clock sleeps, and the **watchdog** watches the program so
it does not run away. Covering four peripherals in one chapter is
impossible — the goal here is to get one example running against each
peripheral's "minimum usable surface", a first look and nothing more; the
deep water is left to Part IV (Chapter 10 PWM, Chapter 12 DMA, Chapter 13
RTC and watchdogs).

**In this chapter you will learn:**

- Period overflow and the update interrupt of the BTIM basic timer: how
  PSC/ARR are chosen automatically, and how to hook the ISR;
- DMA memory→memory transfer: the channels and triggers behind a single
  `mem_to_mem` call;
- RTC clock source selection (LSE first, LSI as fallback) and BCD date/time
  read/write;
- The IWDT's timeout-reset loop and its key-value protocol, plus the concept
  of "windowed feeding" in the WWDT;
- The CW32-specific `R1W0` flag-clearing convention (write 0 to clear, not
  write 1).

## 6.1 Timers: Let the Hardware Count the Beats

BTIM (basic timer, manual §14) is the simplest of the three timer families:
a 16-bit auto-reload counter plus a 16-bit prescaler, with the counting
clock taken straight from PCLK — **none of STM32's ×2 rules** (manual
Figure 14-5). The overflow-frequency formula:

```text
Overflow frequency = PCLK / ((PSC+1) × (ARR+1))
```

The HAL's `Btim` takes over the business of picking the divider (from
`calc_period` in cw32l012-hal `src/btim.rs`): the policy is **pick the
smallest PSC such that ARR ≤ 0xFFFF** — the larger ARR is, the smaller the
relative quantization error. The 2 Hz @ 4 MHz solution is backed by a unit
test: PSC=30 (÷31), CK_CNT ≈ 129.0 kHz, ARR=64515, actual overflow
frequency 1.99997 Hz.

Starting the timer and hooking the interrupt takes four steps (corresponding
to segment A of this chapter's example):

```rust,ignore
let mut tim = Btim::new(p.btim1, &mut rcc);          // enables the APBEN2[2] gate automatically
tim.start_periodic(clocks.pclk_hz, 2)?;              // sets PSC/ARR, clears leftover flags, starts
tim.enable_update_irq(true);                         // enables the update interrupt at the peripheral level (DIER.UIE)
unsafe { runtime::Interrupt::Btim1.enable(); }       // lets IRQ20 through at the NVIC level
```

Note that an interrupt has **two layers of switches**: what `start_periodic`
turns on is the peripheral-internal `DIER.UIE`; the NVIC-level
`Interrupt::Btim1` (IRQ20, vector symbol `BTIM1_irq`) still needs its own
`enable()`, and finally `cpsie i` enables interrupts globally. Miss any one
layer and the interrupt never reaches the ISR. With the interrupt hooked up,
the main loop can go to sleep in `wfi` — the CPU parks in its sleep state,
the hardware wakes it into the ISR when the timer overflows, and it sleeps
again on return. The same blink: in the busy-wait version (Chapter 4) the
CPU spends all its time spinning through counts, while the interrupt version
burns almost nothing.

The ISR itself is declared with the HAL's `interrupt!` macro (full usage in
the 6.5 example): the macro does exactly one thing — attach the vector-table
symbol name `BTIM1_irq` to your interrupt handler function (in the linker
script `memory.x` every vector is a weak default
`PROVIDE(XXX_irq = DefaultHandler)`, and a strong symbol of the same name
overrides it automatically). Inside the ISR there are two pieces of routine
business: **clear the flag**, **do the work**. And clearing the flag runs
straight into CW32's signature temperament:

> **`ICR` is an R1W0 register** (reset value 0xFFFF_FFFF): writing 1 does
> nothing, **writing 0 is what clears**. To clear the update flag, write
> `ICR_CLEAR_UIF = !1` — bit0 forced to 0, all the other bits 1. The
> "write 1 to clear" muscle memory carried over from STM32 will leave the
> flag set forever here, and the interrupt re-enters the instant it exits.

A word on the other two timer families while we are here: GTIM
(general-purpose, 4 instances × 4 PWM channels) and ATIM (advanced,
complementary PWM + dead time + brake). The ecosystem repo's `gtim_pwm`
example has been verified on a real board with a 1 kHz PWM breath
(TESTING.md: ARR=3999/PSC=0, CNT sweeping the full 0..3999); PWM's main
course is Chapter 10.

## 6.2 DMA: Moving Data Without the CPU

DMA (manual §8) has 4 independent channels; the lower the channel number,
the higher the priority (§8.5.4). One transfer is configured following the
manual's 14-step flow of §8.4.1, which the HAL compresses into three calls:

```rust,ignore
let mut dma = Dma::new(p.dma, &mut rcc);   // enables the AHBEN[0] gate automatically
let ch = dma.channel(3);                   // channel 3
ch.configure(&cfg);                        // DISABLE → clear flags → write registers → EN
ch.start();                                // software trigger (TRIG.SOFTSRC)
ch.wait()?;                                // block until the transfer completes
```

The `mem_to_mem` used in the example is the packaged version of this path:
the generics pick the bus width from the element size automatically
(u32 → 32 bits), channel 3 is picked automatically, software trigger,
blocking wait; after the transfer you verify with a word-for-word
comparison — **the CPU only issues the order at the start and inspects the
result at the end; the moving in between proceeds entirely without it**. The
real value lies in hardware triggers: swap the trigger source to ADC
conversion complete (EOC) and every sample lands in memory by itself — that
is the main course of Chapter 12. This is exactly how the P2 phase of the
ecosystem repo's `dma_demo` runs: a DAC→ADC on-chip loopback, with DMA
moving 256 16-bit results; measured `P2_REMAIN=0`, `P2_STATUS=5` (Table 8-2,
"transfer complete").

Besides R1W0, DMA has two more easy-to-trip hardware behaviors (documented
in full in the `dma.rs` doc comments):

- **`CNT.REPEAT` is cleared to 0 by hardware after each round of transfers**
  (§8.5.2) — back-to-back multi-round transfers must call `configure()`
  again each round; only changing the count without reconfiguring does
  nothing;
- **The software trigger `SOFTSRC` self-clears once done** — to transfer
  again, write it again.

## 6.3 RTC: It Keeps Time Even When the Main Clock Sleeps

The RTC (manual §13) is a BCD calendar: year/month/day/weekday plus
hours/minutes/seconds, with leap years and month lengths handled by the
hardware. It hangs on its own clock domain, with four selectable sources
such as LSE (external 32.768 kHz crystal) and LSI (internal ~32.8 kHz RC)
(the `RtcClock` enum). The contract of `Rtc::new` is "if the source you
asked for fails to start, return Err" — the caller decides the fallback.
The example goes LSE first, LSI as fallback:

```rust,ignore
let rtc = match Rtc::new(p.rtc, &mut rcc, RtcClock::Lse) {
    Ok(r) => r,
    Err(_) => match Rtc::new(p.rtc, &mut rcc, RtcClock::Lsi) {
        Ok(r) => r,
        Err(_) => loop { core::hint::spin_loop() },
    },
};
```

The LSE check carries a built-in start-up wait of about 2 s
(`Rcc::enable_lse` busy-waits on `LSE.STABLE`; typical start-up is
200–500 ms); on failure the RTC configuration has not been touched yet, so
falling back to LSI is clean. Measured (TESTING.md `rtc_demo`, 2026-10-23),
**LSE does not start on this board (CLK=2 → the LSI path)** — the StartKit
either has no 32.768 kHz crystal soldered or its load capacitors do not
match. The RTC runs just as well on a board without a crystal: LSI is
factory-TRIMmed (measured 32768 Hz); only its long-term timekeeping accuracy
is not individually calibrated — for accurate time, attach LSE.

The date/time fields are **decimal values** (the module handles the BCD
conversion internally), all fitting in one `DateTime` struct: `year` 0..=99
means 20xx, `weekday` 0=Sunday..6=Saturday. Writing uses `set_datetime`
(which runs a field-validity check first), starting uses `start()`, reading
uses `datetime()` (read twice in a row and take the stable value — the
approach recommended by manual §13.3.6). All RTC register writes must pass
the **unlock key**: write 0xCA and then 0x53 to `KEY`, and write 0xFF
afterwards to lock again — the HAL's `with_unlocked` helper wraps this
procedure up tight; skip the unlock when hand-writing registers and the
write is silently dropped just the same.

One pitfall stepped on in real testing deserves to be remembered
(TESTING.md `rtc_demo`): **in the stopped state, `CR1.WAIT` never clears**.
The manual's §13.3.6 "wait for WAIT" flow only applies to writing DATE/TIME
**while running**; with the clock stopped you can write directly. Following
the running-state flow and waiting for WAIT will wait all the way to a
timeout and return Err — even though the time has in fact already been
written.

## 6.4 Watchdogs: A Bite Before the Program Runs Away

The watchdog's logic is one sentence: a down-counter is running, and if you
do not periodically "feed" it, it resets the chip when it reaches zero. When
the program runs away (an endless loop, stuck inside some wait), the feeding
code no longer executes, and the timeout reset pulls the system back onto
the rails.

**IWDT independent watchdog (§19)** — its counting clock is the independent
RC10K (≈10 kHz), completely isolated from the main clock domain, so it can
still reset the chip when the main clock collapses. Every operation goes
through the **key-value protocol**: `0xCCCC` to start, `0xAAAA` to feed,
`0x5555` to unlock the configuration, `0x5A5A`+`0xA5A5` to stop (debug use
only). The HAL's wrapper hides the keys:

```rust,ignore
let mut wdt = Iwdt::new(p.iwdt, &mut rcc);
wdt.start(TimeoutMs(1500))?;   // timeout ≈1.5 s (RC10K drifts at the ±% level)
wdt.feed();                    // feed the dog: reload the counter
```

`start` solves the prescaler (4..512, eight steps) and the 12-bit ARR
automatically: 1500 ms → prescaler /4, ARR=3749, nominal timeout exactly
1.5 s; because RC10K itself drifts by a few percent, `TimeoutMs` is an
approximation — leave margin in any safety-relevant design: the example's
feed interval of 600 ms is less than half the timeout.

**WWDT window watchdog (§20)** — on top of the IWDT it adds "**you must not
feed too early**": a 7-bit down-counter, feeding must land inside the window
[WINR, WCNT] (§20.3.4); feeding early also triggers a reset. This guards
against a different kind of accident: the program is not dead, but it has
lost its rhythm (some loop returns early and runs one extra pass, say) — a
fixed-interval feed still gets fed, whereas windowed feeding exposes the
broken rhythm. The HAL's `Wwdt::feed()` first reads the current count and
**only writes if it is inside the window** — if it is too early it returns
`Err(FeedError::TooEarly)` instead of letting the chip reset; `feed_forced()`
is the real write, for use as a control experiment. Measured with the
ecosystem repo's `wwdt_demo` (TESTING.md, 2026-10-17~19): window
[98.3, 200.7] ms, of 50 rounds every "early feed" was intercepted
(`LAST_CNT=0x64` at the early moment > WINR=0x58 — judged correctly), every
normal feed succeeded, and the program took zero resets. This chapter's
example uses IWDT only; WWDT is left to Chapter 13.

## 6.5 The Examples: Two Files, Zero Wiring

Four peripherals packed into two examples, neither needing any wiring. Build
commands:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch06_tim_dma -p cw32-book-hal
cargo build --release --target thumbv6m-none-eabi --example ch06_rtc_iwdt -p cw32-book-hal
```

**ch06_tim_dma.rs** — segment A is the BTIM1 2 Hz update-interrupt LED blink
(main loop in `wfi`); segment B is a DMA memory→memory transfer of 16 u32s
with word-by-word verification, the results landing in the `DMA_OK`/
`DMA_BAD` statics (readable from a debugger):

```rust
//! Chapter 6 example (part 1): BTIM1 timer-interrupt blink + DMA memory
//! transfer (zero wiring).
//!
//! Segment A (timer): BTIM1 overflow interrupt at 2Hz; the ISR flips PC13
//! via TOG and the main loop sleeps in `wfi` — same source as cw32l012-hal
//! examples/timer_blink.rs.
//! Segment B (DMA): `Dma::mem_to_mem` moves 16 u32s from SRC into DST and
//! verifies word by word; the result lands in the `DMA_OK`/`DMA_BAD`
//! statics (readable from a debugger) — the transfer path is the same as
//! the P1 of examples/dma_demo.rs (the original example uses 64 words).
//!
//! What you see after building: the PC13 level flips every 500ms
//! (overflow at 2Hz) = the interrupt path is alive; the DMA result needs a
//! debugger attached to read the statics.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch06_tim_dma -p cw32-book-hal
//! Status: verified via the same-source examples — the two segments
//!       correspond to the TESTING.md verified-on-hardware table entries
//!       timer_blink (PC13 toggle sequence 0,0,1,0,0,1,1,0) and dma_demo
//!       (P1_OK=64, P1_BAD=0xFFFF_FFFF); this combined file itself only
//!       compiles, not yet run on hardware.

#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, write_volatile};

use cw32_book_hal as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use cw32l012_hal::{
    btim::Btim,
    dma::Dma,
    gpio::{Level, Pin, PortId},
    pac,
    rcc::Rcc,
    runtime, Peripherals,
};

/// Interrupt count (observable from a debugger)
static mut ISR_COUNT: u32 = 0;
/// DMA source data (generated at compile time)
static SRC: [u32; 16] = {
    let mut a = [0u32; 16];
    let mut i = 0;
    while i < 16 {
        a[i] = 0xA5A5_0000 ^ (i as u32);
        i += 1;
    }
    a
};
/// DMA verification: number of word-identical elements (should = 16)
static mut DMA_OK: u32 = 0;
/// DMA verification: index of the first mismatch (0xFFFF_FFFF = all match)
static mut DMA_BAD: u32 = 0xFFFF_FFFF;

#[unsafe(no_mangle)]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };

    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks();

    let _led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);

    // ---- Segment B: DMA memory→memory (transfer first, interrupts later) ----
    {
        let mut dma = Dma::new(p.dma, &mut rcc);
        let mut dst = [0u32; 16];
        // Blocking transfer to completion (software trigger, 32-bit width,
        // channel 3 picked automatically); even on Err we do not exit
        // early — the word-by-word check below records the result
        // faithfully into DMA_OK/DMA_BAD
        let _ = dma.mem_to_mem(&SRC, &mut dst);
        let mut ok = 0u32;
        let mut bad = 0xFFFF_FFFFu32;
        for i in 0..16 {
            if dst[i] == SRC[i] {
                ok += 1;
            } else if bad == 0xFFFF_FFFF {
                bad = i as u32;
            }
        }
        unsafe {
            write_volatile(addr_of_mut!(DMA_OK), ok);
            write_volatile(addr_of_mut!(DMA_BAD), bad);
        }
    }

    // ---- Segment A: BTIM1 @2Hz update-interrupt blink ----
    let mut tim = Btim::new(p.btim1, &mut rcc);
    match tim.start_periodic(clocks.pclk_hz, 2) {
        Ok(_) => {}
        Err(_) => loop {
            core::hint::spin_loop();
        },
    }
    tim.enable_update_irq(true);

    // Enable BTIM1 (IRQ20) at the NVIC level, then enable interrupts globally
    unsafe {
        runtime::Interrupt::Btim1.enable();
        core::arch::asm!("cpsie i");
    }

    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

// BTIM1 update interrupt: clear the flag → flip PC13 (atomic TOG write) → count
cw32l012_hal::interrupt!("BTIM1_irq", btim1_isr, {
    unsafe {
        let p = Peripherals::steal();
        p.btim1.icr.write(pac::btim::ICR_CLEAR_UIF);
        p.gpioc.tog.write(1 << 13);
        ISR_COUNT += 1;
    }
});
```

What you see after flashing: the PC13 level flips every 500 ms — direct
evidence that "the interrupt path is alive" (the CPU is sitting in `wfi`,
and the LED still blinks, so it must be the hardware timer waking it). The
DMA transfer result has to be read from the SRAM statics with a debugger:
`DMA_OK` should be 16, `DMA_BAD` should be 0xFFFF_FFFF (no mismatch);
`ISR_COUNT` climbs by 2 per second.

**ch06_rtc_iwdt.rs** — segment A starts the RTC at 23:59:50 and waits
12 seconds; the read-back should already have crossed into the next day,
verifying BCD carry, day rollover, and the weekday increment in one shot;
segment B starts the IWDT with a 1.5 s timeout, feeds the dog 5 times (5
quick PC13 blinks), then deliberately stops feeding and waits for the
watchdog to bite and reset, after which the firmware runs again from the
top:

```rust
//! Chapter 6 example (part 2): RTC day-rollover timekeeping + IWDT watchdog
//! reset loop (zero wiring).
//!
//! Segment A (RTC): LSE first; if it fails to start, fall back to LSI
//! (measured: LSE does not start on this board — no 32.768k crystal
//! soldered or load capacitors mismatched, so it actually runs on LSI);
//! set 2026-10-23 (Friday) 23:59:50, wait 12s after start and read back —
//! it should already have crossed into the next day (Saturday), verifying
//! BCD carry, day rollover and the weekday increment in one shot — same
//! source as the P1 of cw32l012-hal examples/rtc_demo.rs.
//! Segment B (IWDT): 1.5s timeout; the first 5 feeds (each flips PC13 →
//! 5 quick blinks), then deliberately stop feeding → after 1.5s the
//! watchdog resets and the firmware runs again from the top — same source
//! as examples/wdt_demo.rs.
//!
//! Power-up rhythm: LED off for the first ~12s (the RTC is doing the
//! day-rollover test) → 5 quick blinks → off ~1.5s → reset, another round.
//! The timekeeping results are in the CLK/DT_AFTER/TM_AFTER statics
//! (readable from a debugger).
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch06_rtc_iwdt -p cw32-book-hal
//! Status: verified via the same-source examples — the two segments
//!       correspond to the TESTING.md verified-on-hardware table entries
//!       rtc_demo (CLK=2 on LSI, rollover at 00:00:01, weekday incremented)
//!       and wdt_demo (BOOT_ID climbing to 6 → reset back to 1, looping);
//!       this combined file itself only compiles, not yet run on hardware.

#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, write_volatile};

use cw32_book_hal as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    rtc::{DateTime, Rtc, RtcClock},
    wdt::{Iwdt, TimeoutMs},
    Peripherals,
};

/// RTC clock source actually in use (1=LSE, 2=LSI, 0=failed)
static mut CLK: u32 = 0;
/// Date read back after 12s (year<<24 | month<<16 | day<<8 | weekday)
static mut DT_AFTER: u32 = 0;
/// Time read back after 12s (hour<<16 | minute<<8 | second)
static mut TM_AFTER: u32 = 0;
/// Feed count for this boot (climbs from 0 again after each watchdog reset)
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

    // ---- Segment A: RTC day-rollover timekeeping (LSE first, fall back to LSI) ----
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

    // Set 23:59:50 → start → 12s later it should have crossed into the next
    // day (measured 00:00:01, Saturday; nominal :02, the 1s difference is
    // the combined error of LSI timekeeping and the delay baseline, see
    // TESTING.md)
    let t0 = DateTime {
        year: 26,
        month: 10,
        day: 23,
        weekday: 5, // 0=Sunday, 5=Friday
        hour: 23,
        minute: 59,
        second: 50,
    };
    if rtc.set_datetime(&t0).is_err() {
        // Only an invalid field or an RTC sync anomaly can land here; this
        // does not happen on a normal power-up; freezing here makes the
        // problem visible (the LED staying still is also a signal)
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

    // ---- Segment B: IWDT watchdog (1.5s timeout; feed 5 times then stop → reset loop) ----
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
            // Feed only the first 5 times; afterwards deliberately stop → watchdog reset after 1.5s
            if BOOT_ID <= 5 {
                led.toggle();
                wdt.feed();
            }
        }
        delay.delay_ms(600); // feed interval 600ms < 1.5s
    }
}
```

Power-up rhythm: for the first ~12 s the LED is off (the RTC is doing the
day-rollover test) → 5 quick blinks → off ~1.5 s → reset, and another
round. `BOOT_ID` counts the feeds since this boot; sampling it with a
debugger shows it climbing 0→5 and then suddenly jumping back to 1 — that
is the "stopped feeding → watchdog reset" scene in the flesh. The
timekeeping results live in the three statics `CLK` (1=LSE, 2=LSI)/
`DT_AFTER`/`TM_AFTER`.

## 6.6 Verification on Hardware

**Status: verified via the same-source examples (per segment) + the combined
files themselves compile, not yet run on hardware.** The two segments of
each example are each sourced, line for line, from already-measured examples
in the ecosystem repo:

- Segment A of `ch06_tim_dma` is **line-for-line identical in source** to
  cw32l012-hal's `examples/timer_blink.rs` (comments added only); segment
  B's transfer path is the same as the P1 of `examples/dma_demo.rs` (a
  16-word reduction; the original example uses 64 words).
- Segment A of `ch06_rtc_iwdt` is sourced from the P1 of
  `examples/rtc_demo.rs` (alarm phase removed); segment B is sourced from
  `examples/wdt_demo.rs` (the same BOOT_ID rhythm).

The measured records for the corresponding ecosystem entries (cw32l012-hal
`TESTING.md`):

| Example | What was verified | Key readings | Measured on |
|------|----------|----------|----------|
| timer_blink | BTIM1 2 Hz interrupt + wfi | PC13 toggle sequence 0,0,1,0,0,1,1,0 | 2026-10-17~19 |
| dma_demo | DMA memory / ADC hardware trigger | P1_OK=64, P1_BAD=0xFFFF_FFFF; P2_REMAIN=0 | 2026-10-23 |
| rtc_demo | RTC day rollover + alarm | CLK=2 (LSI), rollover at 00:00:01, weekday incremented | 2026-10-23 |
| wdt_demo | IWDT timeout reset | BOOT_ID climbs to 6 → resets back to 1, looping | 2026-10-17~19 |
| wwdt_demo | WWDT windowed feeding | EARLY=51/FEEDS=50 consistent, LAST_CNT=0x64 correctly rejected | 2026-10-17~19 |

The meaning of "the combined files themselves compile, not yet run on
hardware", stated plainly: the two `.rs` files of this chapter are new files
merging already-verified segments, and **have never been flashed to a board
in this combined form**. On the first hardware run, check in order:

1. `ch06_tim_dma`: first check whether the LED flips every 500 ms (the
   interrupt path), then attach a debugger and read `DMA_OK`/`DMA_BAD` (the
   transfer path); the two segments are independent of each other;
2. `ch06_rtc_iwdt`: first read `CLK` to confirm the clock source (expected
   to be 2=LSI on this board), then wait 12 s and read `DT_AFTER`/
   `TM_AFTER` (expected rollover at 00:00:01 ± 1 s), and finally watch
   BOOT_ID's reset loop;
3. A deviation of about 1 s between the RTC read-back and the expectation is
   normal (the combined error of LSI timekeeping plus the delay baseline;
   see the TESTING.md `rtc_demo` notes);
4. If `DMA_OK` < 16: check whether the channel number of `mem_to_mem` was
   touched — the example relies on the HAL picking channel 3 automatically.

## 6.7 Common Pitfalls

- **The interrupt's three layers of switches**: `DIER.UIE` (peripheral),
  NVIC `enable()` (IRQ20), `cpsie i` (global) — miss one layer and the
  interrupt never gets in; before `wfi`, all three must be in place.
- **R1W0 flag clearing**: CW32's `ICR` clears on 0, and writing 1 does
  nothing (reset value 0xFFFF_FFFF). The STM32 "write 1 to clear" habit
  never clears anything — BTIM's `ICR_CLEAR_UIF = !1` and DMA's
  `icr.write(!(1<<0 | 1<<1))` are both write-0.
- **`Peripherals::steal()` inside the ISR**: `take()` was already consumed
  in `main`, so the ISR can only `steal()` another copy; register conflicts
  within the same interrupt are the author's own responsibility.
- **DMA's REPEAT and SOFTSRC are both auto-cleared by hardware**: continuous
  transfers need a fresh `configure()` every round, and multi-round software
  triggers need a fresh `start()` every round.
- **DMA source/destination addresses must not be the DMA/FLASH/RAM
  controllers' own registers** (manual §8.8.6 note); the address increment
  is determined by the width (8/16/32 bits → 1/2/4 bytes).
- **RTC unlock key 0xCA→0x53**: every register write needs the unlock and a
  relock afterwards; in the stopped state, do not wait on `CR1.WAIT` — it
  never clears; only writes while running need the wait.
- **LSE on this board does not start**: no 32.768k crystal soldered (or load
  capacitors mismatched); the RTC example automatically falls back to LSI
  (`CLK=2`) — that is expected behavior, not a fault.
- **The IWDT timeout is an approximation**: RC10K drifts at the ±% level;
  `TimeoutMs(1500)` is nominally 1.5 s but may actually run anywhere in
  1.45–1.55 s — leave plenty of margin in the feed interval.
- **Feeding the WWDT early resets the chip**: the feed must land inside the
  window; the HAL's `feed()` intercepts an early attempt with
  `Err(TooEarly)`, and only `feed_forced()` really writes.
