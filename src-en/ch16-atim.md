# Chapter 16 Complementary PWM and Braking: the ATIM Advanced Timer

In this chapter you will learn:

- Why a half-bridge needs **complementary PWM + dead time** — and how the
  shoot-through short circuit that destroys power stages actually happens;
- The three things ATIM adds over GTIM: the **MOE master output enable**,
  the **BDTR dead-time generator**, and the **BRK brake**;
- The four-segment DTG dead-time encoding, and how `encode_dtg` translates
  "250 µs" into 0xFF;
- The right way to wire a brake input — including a real-hardware
  "no waveform at all" trap.

This chapter's example (blocking HAL) corresponds to the `atim_deadtime`
entry in the hal repo's TESTING.md — a **hardware-verified recipe**: the
BDTR read-back, brake shutdown and full-range CNT all have measured data.
The embassy `atim` driver will be added to this chapter once
embassy-cw32 0.4.0 provides it.

## Why a single PWM channel is not enough for a half-bridge

Chapter 10's GTIM produces one "adjustable duty cycle" square wave — fine
for blinking LEDs and steering servos. Motor power stages, however, are
usually built as a **half-bridge**: the high-side transistor Q1 connects
to the supply rail, the low-side Q2 to ground, and the load hangs between
them.

```text
        VCC          VCC
         |            |
        [Q1]        [Q2]   ← must never conduct together!
         |____________|
              |
            load
```

Q1 and Q2 **must never conduct at the same time** — that shorts the
supply rail to ground ("shoot-through") and destroys the power stage in
an instant. So the two control signals must satisfy:

- **Complementary**: while Q1 turns off, Q2 turns on so the load current
  has a path;
- **Dead time**: but the hand-off can't be seamless in physics —
  transistors switch off with delay. If Q2 conducts before Q1 has fully
  turned off, shoot-through still happens. A **both-off** gap (the dead
  time) must sit between the two edges, typically hundreds of ns to a few
  µs.

GTIM cannot do this: one output pin per channel. The ATIM (advanced
timer) provides a complementary pair CHy/CHyN per channel and inserts
dead time **in hardware** — software only writes duty cycles, and the
DTG register configures the gap once.

## What ATIM adds on top of GTIM

ATIM's counting core (PSC/ARR/CCR/shadow registers/ARPE preloading,
§17.1 of the user manual) is isomorphic to GTIM — everything in
Chapter 10 still applies. The additions concentrate in one register plus
one more:

| Capability | Register/bit | Meaning |
|------|-----------|------|
| Master output enable | BDTR.MOE | **The master switch for every PWM pin**. Without it, CCER can be perfect and the pins stay flat |
| Dead-time generator | BDTR.DTG[7:0] | Both-edge off-gap inserted between a complementary pair, in hardware |
| Brake BRK | BDTR.BKE/BKP/BKF | When the external pin goes active, hardware **asynchronously clears MOE** — outputs enter the safe state immediately |
| Idle-state outputs | BDTR.OSSI/OSSR | During brake/idle the pins output a *defined* level instead of floating |
| Repetition counter | RCR | One update event per N periods (for synchronized three-phase sampling) |

The key mental model: **MOE is the final gate**. The usual init order is:
pins to AF → PSC/ARR/CCR → CCER channel enables → DTG dead time → and
*last*, MOE. Any earlier mistake still cannot shoot through the half
bridge (and with the brake configured, BRK remains the backstop).

The blocking HAL wraps this order in `Atim::start_pwm_complementary`:
one call takes frequency/duty/dead time. `start_pwm` is the
non-complementary single-ended variant (same timer, usable as a
"deluxe GTIM" when you don't drive a half-bridge).

## Dead time: the four-segment DTG encoding

The CW32L012 DTG encoding (manual §17, same idea as STM32 advanced
timers):

| DTG[7:5] | Dead time (in units of t_DTS) |
|----------|---------------------------|
| 0xx      | DTG[7:0] × t_DTS (1..127, step 1) |
| 10x      | (64 + DTG[5:0]) × 2 × t_DTS (128..254, step 2) |
| 110      | (32 + DTG[4:0]) × 8 × t_DTS (256..504, step 8) |
| 111      | (32 + DTG[4:0]) × 16 × t_DTS (512..1008, step 16) |

`t_DTS = 1/f_DTS`, `f_DTS = PCLK / CKD` (CR1.CKD; dead time does **not**
pass through PSC — PSC only divides the counter clock. The HAL fixes
CKD=1, i.e. t_DTS = t_PCLK).

Three things to note:

- **Dead time is not linear**: small values have 1 t_DTS resolution;
  large values relax to 2/8/16 t_DTS steps. It is the compression trick
  that fits 1008 settings into an 8-bit field.
- **The ceiling is 1008 t_DTS**: at 4 MHz PCLK that is 252 µs. Anything
  longer cannot be encoded — `encode_dtg` returns `None` →
  `AtimError::DeadTimeTooLong`.
- **Round up**: the HAL's `encode_dtg(pclk_hz, ckd_div, dead_ns)` rounds
  the requested nanoseconds **up** to the nearest encodable step — a
  too-short dead time means shoot-through; a too-long one just costs a
  little efficiency. The example requests 250 µs and encodes to the 0xFF
  step (1008 t_DTS ≈ 252 µs).

## The brake BRK: hardware asynchronously clears MOE

The brake is ATIM's emergency stop. With BDTR.BKE enabled, the moment
the BRK pin goes active (polarity in BKP), hardware **immediately**
clears MOE — no software involvement, no waiting for an update event;
every PWM pin outputs the OSSI-defined safe state. Software recovers by
calling `re_enable_output()` to set MOE again (the BDTR.BIF flag is
readable: BIF=1 means a break happened; it cannot be cleared while the
BRK source is still active).

**The real-hardware trap** (from the hal repo's TESTING.md): the brake
input PA06 must be configured with `into_af_input(Af::Af7)` — the signal
must enter the timer's brake circuit as an **AF-reused input**. A plain
GPIO input or a floating pin makes BRK sample low → brake latches at
power-up → MOE cleared by hardware → no waveform, and BDTR reads back
0x14FF instead of the expected 0x94FF. Hours of "why is there no
waveform" debugging, one wrong pin mode.

## Example

`examples/hal/examples/ch16_atim.rs`: PA08=CH1 / PA15=CH1N (both AF7,
manual Table 9-2) output a 2 kHz complementary PWM at 30% duty with a
250 µs dead time (encoded to 0xFF); PA06=BK is configured as an AF7
reused input, pulled up, active-low brake. All observables live in
statics readable by probe-rs:

```rust
//! Chapter 16 example: ATIM complementary PWM + dead time + brake
//! (PA08=CH1, PA15=CH1N, PA06=BK).
//!
//! Same-source as cw32l012-hal examples/atim_deadtime.rs (statics add
//! `#[used]`+`#[unsafe(no_mangle)]` so probe-rs can read the symbols
//! directly, plus edition 2024 attribute syntax). 2kHz, 30% duty,
//! 250µs dead time (rounded up to DTG=0xFF at 4MHz PCLK, i.e. 1008
//! t_DTS ≈ 252µs).
//!
//! How to verify (a debugger reading statics is enough):
//! - `REG` read-back (ARR<<16 | PSC) — expected 0x07CF_0000 (ARR=1999, PSC=0)
//! - `CCR` read-back — expected 600 (2000×30%)
//! - `BDTR` read-back — expected 0x94FF (MOE=1, OSSI=1, BKE=1, DTG=0xFF)
//! - `CNT_MIN/MAX` — expected to sweep the full 0..1999 range
//! - With a scope: PA08/PA15 complementary square waves, edge-to-edge
//!   gap ≈252µs, never overlapping; pulling PA06 (ATIM_BK) low shuts
//!   both off at once (MOE cleared by hardware, `BDTR` reads 0x14FF),
//!   `re_enable_output()` restores.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch16_atim -p cw32-book-hal
//! Status: same-source verified — corresponds to the hal repo's
//!       TESTING.md measured-entry atim_deadtime (REG=0x07CF0000,
//!       CCR=600, BDTR=0x94FF, CNT sweeping the full range, PA06 brake
//!       shutdown); this composed file itself is compile-only.

#![no_std]
#![no_main]

use cw32_book_hal as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use cw32l012_hal::{
    atim::{Atim, AtimChannel},
    gpio::{Af, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

/// (ARR<<16 | PSC) configuration read-back: expected 0x07CF_0000 at 2kHz@PCLK 4MHz
#[used]
#[unsafe(no_mangle)]
pub static mut REG: u32 = 0;
/// Latest CCR read-back: expected 600 at 30% duty
#[used]
#[unsafe(no_mangle)]
pub static mut CCR: u16 = 0;
/// BDTR read-back (MOE/DTG/brake bits): expected 0x94FF
#[used]
#[unsafe(no_mangle)]
pub static mut BDTR: u32 = 0;
/// Minimum CNT sample: expected 0 (the counter really is rolling)
#[used]
#[unsafe(no_mangle)]
pub static mut CNT_MIN: u16 = 0xFFFF;
/// Maximum CNT sample: expected 1999
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

    // Complementary output pins: PA08 = ATIM_CH1, PA15 = ATIM_CH1N (both AF7, manual Table 9-2)
    let _ch1 = Pin::new(p.gpioa, PortId::A, 8, &mut rcc).into_af_output(Af::Af7);
    let _ch1n = Pin::new(p.gpioa, PortId::A, 15, &mut rcc).into_af_output(Af::Af7);
    // Brake input PA06 = ATIM_BK: must go through the AF7 reused input (the
    // brake circuit's filter/polarity lives there; a plain GPIO input cannot
    // sample BRK). An external pull-up keeps it high = no brake; pulling it
    // low = brake. A floating pin lets BRK false-trigger on noise (the
    // real-hardware trap: with a plain input mode MOE gets cleared and the
    // output is dead — BDTR reads back 0x14FF)
    let _bk = Pin::new(p.gpioa, PortId::A, 6, &mut rcc).into_af_input(Af::Af7);

    let mut tim = Atim::new(p.atim, &mut rcc);
    // 2kHz @ PCLK 4MHz: PSC=0, ARR=1999; 30% duty; 250µs dead time
    // (encode_dtg rounds up → DTG=0xFF, 1008 t_DTS ≈ 252µs)
    if tim
        .start_pwm_complementary(AtimChannel::Ch1, clocks.pclk_hz, 2_000, 3_000, 250_000)
        .is_err()
    {
        loop {
            core::hint::spin_loop();
        }
    }
    // Brake: BRK active low (pulling it low → MOE cleared by hardware → safe state)
    tim.enable_brake(false, 0);
    tim.re_enable_output(); // if a power-up glitch already latched the brake, restore the master output here

    unsafe {
        let (arr, psc) = tim.period_config();
        REG = (arr as u32) << 16 | psc as u32;
        CCR = tim.ccr(AtimChannel::Ch1);
        BDTR = tim.bdtr();
    }

    // Runtime observation: the CNT rolling range (proof the counter lives);
    // you can also call set_duty periodically here for a breathing effect —
    // duty takes effect on the next cycle (OCxPE preloading)
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
```

Expected read-backs (all with on-hardware corroboration, see the hal
repo's TESTING.md):

| Static | Expected | Meaning |
|--------|------|------|
| `REG` | `0x07CF_0000` | ARR=1999, PSC=0 (4M/2k = 2000 steps) |
| `CCR` | `600` | 2000 × 30% |
| `BDTR` | `0x94FF` | MOE=1, OSSI=1, BKE=1, DTG=0xFF |
| `CNT_MIN/MAX` | `0` / `1999` | counter sweeps the full period |

With a scope (optional): PA08/PA15 complementary square waves with a
≈252 µs gap between the two edges, never overlapping; pull PA06 low and
both channels shut **simultaneously** (BDTR reads `0x14FF` — the MOE bit
is gone); `re_enable_output()` restores.

## Common Pitfalls

- **Brake pin floating or in plain-input mode**: brake latches at
  power-up, MOE is cleared, output is dead (BDTR reads 0x14FF). Use
  `into_af_input(Af::Af7)` plus an external pull-up for the active-low
  configuration.
- **Dead time over the ceiling**: a request above 1008 t_DTS returns
  `DeadTimeTooLong` — it does not silently clamp to the largest step.
  Better an error than a compromise: a wrong half-bridge parameter burns
  the transistors.
- **MOE forgotten**: if the pins are flat, check BDTR.MOE first. The
  ATIM output chain is comparator → CCER → dead time → **MOE** → pin;
  the first three can be perfect and the closed final gate still leaves
  a flat line.
- **Whole-word BDTR writes clobber siblings**: brake config
  (BKE/BKP/BKF), dead time (DTG) and the master switch (MOE) share one
  register; a later whole-word write stomps the earlier bits. The HAL
  uses read-modify-write (`modify`) internally; if you touch registers
  by hand, remember BDTR is a multi-tenant register.

## Exercises

1. If the example's dead-time request changes to 300 µs (above the
   1008 t_DTS = 252 µs ceiling), what does `start_pwm_complementary`
   return? How can you legitimately obtain a 300 µs dead time at 4 MHz
   PCLK? (Hint: CKD can divide by 2/4 — the second `encode_dtg`
   parameter.)
2. With BDTR.AOE (automatic output enable) = 1, the next update event
   after a brake restores MOE automatically. For a motor application,
   do you prefer AOE=0 (software must explicitly recover) or 1? What is
   the risk of each?
3. With RCR (repetition counter) = 4, update events come at 1/5 the
   rate. What does that buy a three-phase FOC controller that refreshes
   all duty cycles once per period?

The embassy complementary PWM (`atim` module, `wait_update` async
pacing + three-phase complementary) ships with embassy-cw32 0.4.0; the
embassy section will be added to this chapter then.
