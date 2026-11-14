# Chapter 10 Timers and PWM

In this chapter you will learn:

- The GTIM (general-purpose timer) register model: how PSC/ARR/CCR/CCMR
  cooperate to produce one PWM output;
- The constructor parameters of `GtimPwm` and `PwmConfig`, and the semantics
  of the basis-point duty cycle (`duty_bp`);
- Why a duty-cycle change "takes effect the next cycle" — shadow registers
  and ARPE;
- How `wait_update().await` turns the "PWM cycle boundary" into an async
  beat.

## The GTIM Register Model

The CW32L012 has three tiers of timers: BTIM1–3 (basic), GTIM1–4
(general-purpose), and ATIM (advanced). Part IV of this book only expands
GTIM — it best represents the generic model of "timer = counter +
comparator"; ATIM's complementary PWM and dead time get their own
[Chapter 16](ch16-atim.md). The GTIM
register surface is in the *CW32L012 User Manual* §16.

One PWM output is decided by four registers (§16.10):

- **PSC** (prescaler): the timer clock PCLK is first divided by `PSC+1` to
  get the counting clock CK_CNT;
- **ARR** (auto-reload): the counter CNT rolls over across `0..=ARR`, which
  sets the PWM period: `f_pwm = PCLK / ((PSC+1) × (ARR+1))`;
- **CCR** (capture/compare): CNT is compared against CCR at every count;
- **CCMR/CCER** (capture/compare mode/enable): how the comparison result
  drives the output pin.

`embassy-cw32`'s `GtimPwm` is hard-wired to **edge-aligned PWM mode 1,
active-high** (the output is active while CNT < CCR; the OCyM bitfield of
CCMRx, §16.10). This register recipe is line-for-line same-source as the
blocking `cw32l012-hal`'s `gtim.rs` — the latter has already been verified
on hardware by lighting an LED (hal repo TESTING.md: GTIM1 at 1 kHz,
ARR=3999/PSC=0, CNT sweeping the full 0..3999).

`GtimPwm::new` automatically does three things inside: it enables the
`APBEN1.GTIMx` clock gate (§4; GTIM1..4 = bit6/7/11/12, KEY write-protected),
configures the pin as an AF alternate output, and picks PSC/ARR and starts
the counter. The selection policy is **the smallest PSC such that ARR is as
large as possible** — the larger ARR is, the finer the duty-cycle
quantization step. For example, generating 1 kHz from a 4 MHz PCLK: 4000
counts ≤ 65536, so PSC=0 and ARR=3999.

## Duty Cycle: Full Scale in Basis Points

`PwmConfig::duty_bp` and `set_duty_bp(duty_bp)` both express the duty cycle
in **basis points** (parts per ten thousand): 0 = constant low, 10000 =
constant high, 5000 = 50%. Full scale is fixed at 10000 rather than "ARR+1"
because the `embedded-hal` `SetDutyCycle` trait reports the maximum duty
cycle as a u16, and ARR+1 can reach 65536, which does not fit — with the
fixed basis-point scale, `GtimPwm` also implements that trait, so
breathing-LED/servo drivers from the `embedded-hal` ecosystem plug in
without changing a line.

## ARPE: Why a Duty-Cycle Change Takes Effect the Next Cycle

Writing CCR does not immediately change the output waveform — **OCyPE
preload** is enabled in CCMR (the OCyPE bit of CCMRx, §16.10), so the value
you write first lands in a shadow register and is swapped in wholesale at
the next update event (CNT returning to zero). This makes each duty-cycle
modification **atomic**: no half-wrong cycle ever appears on the waveform.
CR1.ARPE does the same for ARR (§16.3.1.4). The side effect: by the time
`set_duty_bp` returns, the waveform has not changed yet. For a "one step
per cycle" breathing effect you must wait for the update event — which is
exactly what `wait_update` is for.

## wait_update: Turning the Cycle Boundary into an Async Beat

`GtimPwm::wait_update().await` waits for one real update event (ISR.UIF),
and the moment it returns is a PWM cycle boundary. The implementation is
the standard embassy pattern: on entry it first clears any leftover UIF
(the EGR.UG written by `GtimPwm::new` during configuration sets UIF once as
a side effect, §16.10.5), enables DIER.UIE only while waiting (§16.10.4),
and drops the enable bit immediately once the interrupt wakes it (anti-storm
discipline, identical to the UART driver). It depends on the GTIM1
interrupt (IRQ16) being bound to `timer::Gtim1Irq` via `bind_interrupts!` —
**if you only use PWM and never call `wait_update`, you do not need to bind
any interrupt at all**.

GTIM3/GTIM4 share one interrupt line (IRQ18 `GTIM34`); the driver provides
a `Gtim34Irq` marker with paired dispatch. This example only uses GTIM1, so
it does not come into play.

## The Example

`examples/embassy/examples/ch10_pwm.rs`: GTIM1_CH1 output on PB04 (AF6,
manual Table 9-2), 1 kHz, duty cycle sweeping 10%→90%→10% with one step per
PWM cycle; PC13 toggles every 500 update events (= a 1 Hz heartbeat at the
1 kHz beat). Observables land in SRAM statics, readable with probe-rs
(addresses per the `nm` output).

```rust
//! Chapter 10 example: GTIM1 single-channel PWM (PB04, 1kHz, duty cycle
//! sweeping 10%→90%).
//!
//! Each PWM cycle is paced by `wait_update().await` (update interrupt UIF +
//! waker); PC13 toggles every 500 update events (= a 1Hz heartbeat at the
//! 1kHz beat).
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch10_pwm -p cw32-book-embassy
//! Status: compiles, not yet run on hardware (same state as the embassy
//!       version's timer_pwm); the register recipe is same-source as the
//!       blocking gtim_pwm and verified on hardware (ARR=3999/PSC=0, CNT
//!       sweeping the full range, see the hal repo's TESTING.md).
//!       First-flash checklist in the Chapter 10 text.

#![no_std]
#![no_main]

use embassy_executor::main;

use cw32_book_embassy as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use embassy_cw32::{
    Config, Peripherals, SYSCTRL_KEY,
    gpio::{Level, Output, Pin},
    timer::{GtimPwm, PwmChannel, PwmConfig},
};

// wait_update needs the GTIM1 update interrupt (IRQ16); pure PWM usage needs no interrupt bound
embassy_cw32::bind_interrupts!(struct Irqs {
    GTIM1 => embassy_cw32::timer::Gtim1Irq;
});

/// (ARR<<16 | PSC) configuration read-back: expected 0x0F9F_0000 at 1kHz@PCLK 4MHz
#[used]
#[unsafe(no_mangle)]
pub static mut PERIOD: u32 = 0;
/// Duty-cycle read-back (basis points, back-computed from CCR/ARR)
#[used]
#[unsafe(no_mangle)]
pub static mut DUTY_RB: u16 = 0;
/// Number of completed wait_update calls (expected ≈1000/s)
#[used]
#[unsafe(no_mangle)]
pub static mut UPDATES: u32 = 0;
/// Error code: 0=none, 1=construction failed (frequency out of range / duty out of bounds; the constants in this example never trigger it)
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // reset defaults: SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // GPIOB (PB04 PWM pin) + GPIOC (PC13 LED) in one AHBEN write: the L012 is
    // KEY write-protected and whole-word written; two separate writes clobber
    // each other's enable bits (B=5/C=6)
    p.sysctrl
        .ahben
        .write(SYSCTRL_KEY | (1 << 5) | (1 << 6));
    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    // PWM: PB04 = GTIM1_CH1 (AF6, manual Table 9-2); 1kHz, initial duty 10%
    let mut pwm = match GtimPwm::new(
        p.gtim1,
        Pin::new(p.gpiob, 4),
        6,
        PwmConfig {
            channel: PwmChannel::Ch1,
            freq_hz: 1_000,
            duty_bp: 1_000, // basis points: 10000 = 100%
        },
    ) {
        Ok(t) => t,
        Err(_) => {
            unsafe { ERR = 1 };
            loop {
                core::hint::spin_loop();
            }
        }
    };

    let (arr, psc) = pwm.period_config();
    unsafe { PERIOD = (arr as u32) << 16 | psc as u32 };

    // Duty cycle sweeps 10%→90%→10% in a loop (5% per step, one step per PWM cycle)
    let mut duty: u16 = 1_000;
    let mut dir_up = true;
    loop {
        // Direct basis-point control (ARPE preload → takes effect the next update cycle)
        pwm.set_duty_bp(duty).ok();
        // Wait one real PWM cycle (woken by the GTIM1 update interrupt; return moment = cycle boundary)
        pwm.wait_update().await;
        unsafe {
            DUTY_RB = pwm.duty_bp();
            UPDATES += 1;
            if UPDATES % 500 == 0 {
                led.toggle(); // 500 at the 1kHz beat = 1Hz heartbeat
            }
        }
        if dir_up {
            duty += 500;
            if duty >= 9_000 {
                dir_up = false;
            }
        } else {
            duty = duty.saturating_sub(500);
            if duty <= 1_000 {
                dir_up = true;
            }
        }
    }
}
```

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch10_pwm -p cw32-book-embassy
```

## On-Hardware Verification

**Status: compiles, not yet run on hardware.** This example is same-source
as the `timer_pwm` example in the embassy-cw32 repo; under that example's
CHANGELOG 0.4.0 timer entry it counts as a new item whose "register recipe
is same-source as the blocking version", and the embassy version itself has
not been flashed yet. **The async path (the UIF interrupt + waker wake-up
of `wait_update`) is the increment to check on first flash** — the blocking
version only ever verified the PWM output of the pure register recipe.

First-flash checklist:

- `PERIOD` should read `0x0F9F_0000` (ARR=3999, PSC=0; 1 kHz @ PCLK 4 MHz);
- `UPDATES` should grow linearly at ≈1000/s (proof that `wait_update` really
  is waiting on the update interrupt rather than spinning idle); PC13 shows
  a 1 Hz heartbeat;
- `DUTY_RB` (back-computed from CCR/ARR) should follow the set value step
  by step;
- Scope/logic analyzer on PB04: a 1 kHz square wave with the duty cycle
  breathing between 10%↔90%. If your board does not bring out GTIM1_CH1,
  swap the channel/pin/AF for any usable set from Table 9-2 (e.g.
  GTIM1_CH3 @ PA03 = AF6).

## Common Pitfalls

- **A whole-word AHBEN write clobbers other enable bits**: the L012's
  SYSCTRL gates are KEY write-protected, and `ahben.write(KEY | bits)` is a
  whole-word write — in the example, GPIOB and GPIOC must be written in one
  shot; writing them in two passes clobbers whichever enable bit went first
  (KEY semantics in §4.7).
- **`wait_update` without a bound interrupt = hung on the first cycle**: the
  NVIC line falls to the weak default vector, and once UIF is set nobody
  handles it. Either bind `Gtim1Irq` with `bind_interrupts!` or do not use
  `wait_update`.
- **Only 10000 means constant high**: internally `set_duty_bp(10_000)` puts
  CCR at ARR+1, so CNT is always below CCR and the output is permanently
  active; at 9999 there is still one count of low-level glitch per cycle.
- **Pin and channel must match**: with `PwmChannel::Ch1` on PB04 the AF
  must be 6 (Table 9-2); with a wrong AF the GTIM counts away normally but
  no waveform appears on the pin.
- **Realizable-frequency boundaries**: for `f > PCLK/2` or
  `f < PCLK/65536²` the constructor returns `PwmError::OutOfRange` (no
  PSC/ARR combination can save it); at the reset-default 4 MHz PCLK,
  common frequencies like 1 kHz have enormous margin.
