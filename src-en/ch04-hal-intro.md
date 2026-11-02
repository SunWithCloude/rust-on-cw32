# Chapter 4 Meeting cw32l012-hal

In Chapter 2, lighting a single LED took five hand-written register steps;
starting with this chapter we switch to the **blocking HAL** `cw32l012-hal` —
the same blink, with only five lines left in the main code.

**In this chapter you will learn:**

- The design trade-offs of `cw32l012-hal`: zero external dependencies, a
  built-in runtime, why the panic handler is not inside the HAL, and how this
  book tucks it into the example crate's lib.rs;
- The "once" semantics of `Peripherals::take()` and the peripheral ownership
  model;
- A segment-by-segment close reading of `ch04_blink_pc13.rs`: the clock
  snapshot, the `0x5A5A` key-protected GPIO gating write, and exactly which
  reset leftovers `into_output` clears;
- Where `Delay` gets its timing reference, and why it drifts after you change
  the clock frequency.

## 4.1 Crate Positioning and Design Philosophy

`cw32l012-hal` is a `no_std` hardware abstraction layer for Wuhan Xinyuan's
CW32L012, with the register layer (the `pac` module) built into the crate
itself — no external PAC dependency. It makes several explicit trade-offs:

**Zero external dependencies to start.** With no features enabled, the
crate's dependency list is empty — no cortex-m, no cortex-m-rt in
`Cargo.toml`. The register primitives (`Rw`/`Ro`/`Wo`), the vector table,
and the reset initialization are all implemented in-house. The cost is that
you cannot reuse ready-made pieces from the ecosystem; the benefit is a
fully controllable build tree where every line of code can be checked
against the manual.

**A built-in runtime.** The default `runtime` feature provides two
replacements for cortex-m-rt:

1. A 48-entry interrupt vector table (weak symbols that can be overridden;
   IRQ0..31 matches Table 5-1 of the manual);
2. The reset entry point `Reset`, which copies the `.data` section from
   Flash into SRAM and zeroes `.bss`, then calls your `main`.

So the examples **do not** need `cortex-m-rt`, and there is no
`cortex_m_rt::entry` macro — you write `fn main() -> !` directly.

**The panic handler is not provided by the HAL.** `#[panic_handler]` must be
unique across the entire firmware, and the HAL does not make that decision
for you — it belongs to the application side. This book makes the decision
exactly once: the handler lives in the `lib.rs` of the example crate
(`cw32-book-hal`), and each example pulls the lib into linking — and
inherits it automatically — with a single line `use cw32_book_hal as _;`,
the same mechanism as `use panic_probe as _;` in Chapter 3 (the entire
content of panic-probe is, after all, nothing but a panic handler). The
handler itself stays minimal: an endless loop. If you want RTT printing of
panic messages, move the rprintln from Chapter 3 into the lib.

**The feature surface at a glance:**

| Feature | Default | Purpose |
|---|---|---|
| `runtime` | ✅ | Vector table, `.data`/`.bss` initialization, the `interrupt!` macro, NVIC operations |
| `ehal-1` | ✅ | `embedded-hal 1.0` traits (DelayNs / digital / SPI / I2C / PWM), for ecosystem driver compatibility |
| `critical-section-impl` | — | A PRIMASK-based critical-section implementation (only one implementation per firmware) |
| `embedded-io` | — | `embedded_io::Read` / `Write` for `Uart` |
| `defmt` | — | `defmt::Format` for the error types |

Turn everything off (`default-features = false`) and you are back to the
zero-dependency, bare-register form.

**Version basis**: `0.8` has been published on crates.io; this book's
examples use the repository-path version **0.9.x** (`examples/hal/Cargo.toml`
points at the ecosystem repo via `path`). The two have identical APIs; 0.9
mainly adds ecosystem integration and on-hardware verification, and does not
change existing signatures.

## 4.2 The Ownership Model: Peripherals::take()

The HAL collects all the peripheral register blocks into one `Peripherals`
struct:

```rust,ignore
pub struct Peripherals {
    pub sysctrl: &'static pac::Sysctrl,
    pub gpioa: &'static pac::Gpio,
    pub uart1: &'static pac::Uart,
    // ……39 fields in total, one per entry of the peripheral base-address table
}
```

`Peripherals::take()` can only succeed once — the first call returns
`Some(p)`, and every later call returns `None`. This is the standard
"singleton" idiom of embedded Rust: beyond the compile-time type system, it
adds a runtime safety net for the invariant "one peripheral register block,
one owner". The examples in this book all unpack it with `match`:

```rust,ignore
let p = match Peripherals::take() {
    Some(p) => p,
    None => loop { core::hint::spin_loop(); },
};
```

A second call returning `None` can only mean a bug in the code path (say,
`main` being invoked recursively), and the endless loop exposes the problem
explicitly. When an interrupt service routine also needs to touch registers,
use the `unsafe` `Peripherals::steal()` — resolving the ownership conflict
is the ISR author's own responsibility (the timer interrupt in Chapter 6
will use it).

## 4.3 The Example: ch04_blink_pc13.rs

Build command (run from the root of this book's repository):

```bash
cargo build --release --target thumbv6m-none-eabi --example ch04_blink_pc13 -p cw32-book-hal
```

The full example (`examples/hal/examples/ch04_blink_pc13.rs`):

```rust
//! Chapter 4 example: on-board PC13 LED blink (cw32l012-hal, blocking).
//!
//! The full HAL GPIO path: `Pin::new` (internally Rcc::enable_gpio takes the
//! SYSCTRL_AHBEN KEY=0x5A5A protected write) → `into_output` (clears
//! OPENDRAIN/PUR/PDR/ANALOG + resets AF to 0 → DIR=0) → `Output::toggle`
//! (atomic TOG flip).
//! PC13 resets with ANALOG=1 (analog mode); this example verifies that the
//! HAL has cleared it correctly.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch04_blink_pc13 -p cw32-book-hal
//! Status: verified via the same-source example (line-for-line identical to
//!       cw32l012-hal examples/blink_pc13.rs; TESTING.md verified-on-hardware
//!       table: PC13 500ms square wave).
//! Difference: this example is written per edition 2024 as `#[unsafe(no_mangle)]`.

#![no_std]
#![no_main]

use cw32_book_hal as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

#[unsafe(no_mangle)]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };

    // Reset-default clocks: SysClk = HSI = 4MHz, HCLK = PCLK = 4MHz
    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks();

    // On-board LED: PC13 (active low or high depends on the board; toggle works for both wirings)
    let mut led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);

    let mut delay = Delay::new(&clocks);

    loop {
        led.toggle();
        delay.delay_ms(500);
    }
}
```

`#[unsafe(no_mangle)]` is a requirement of the Rust 2024 edition: it exports
`main` under the bare symbol name `main` (the runtime's
`extern "Rust" { fn main() }` has to find it by name), and the `unsafe(...)`
block means the author vouches for the premise that "the symbol name is not
mangled". The PAC examples of Chapter 2 already used the same form.

The rest of this section takes the example apart path by path — five key
paths.

### Segment 1: The clock snapshot — Rcc::new and clocks()

```rust,ignore
let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
let clocks = rcc.clocks();
```

`Rcc::new` takes the two register blocks SYSCTRL and FLASHCTRL and does
three things: it records the reset-default frequencies (SysClk =
HSI = HSIOSC/24 = **4 MHz**, HCLK = PCLK = 4 MHz; the HSI divider encodings
are in manual §4.7.4), and loads the HSIOSC TRIM from the FLASH factory
calibration area (manual §4.4.2, lifting the ±5% RC accuracy to
factory-calibration grade). This example does not change the frequency;
`clocks()` merely takes a snapshot of the three Hz figures
`{ sysclk_hz, hclk_hz, pclk_hz }`, which the baud rate (Chapter 5) and the
delays (end of this segment) both use as their reference.

The clock tree looks like this (bold = the reset-default path):

```text
HSIOSC 96MHz ──/24──> HSI 4MHz ──┐
HSE crystal ──────────────────────┤
LSI ~32.8kHz ─────────────────────┼─> SysClk ──/1──> HCLK ──/1──> PCLK
LSE 32.768kHz ────────────────────┘
```

### Segment 2: Pin::new and the 0x5A5A protected write

```rust,ignore
let mut led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);
```

The first thing `Pin::new(p.gpioc, PortId::C, 13, &mut rcc)` does is turn on
the clock for GPIOC: `rcc.enable_gpio(PortId::C)`. The CW32L012's peripheral
clock gating registers (`AHBEN`/`APBEN1`/`APBEN2`, manual
§4.7.11–4.7.13) are write-protected — the upper 16 bits must be written with
the key value **0x5A5A**, the write-protection key, and only the lower 16
bits carry the enable bits. The HAL folds this detail into a private helper
of `Rcc` (excerpt from cw32l012-hal `src/rcc.rs`):

```rust,ignore
/// Set bits (KEY-protected register; the KEY bits read as 0, the lower 16 bits are the status)
fn set_bits(&self, reg: &pac::Rw<u32>, bits: u32) {
    let v = reg.read() & 0xFFFF;
    reg.write(sc::KEY | v | bits);
}
```

On read-back only the lower 16 bits are taken (the KEY bits read as 0); on
write the key is OR-ed back in — the caller never sees the protection key at
all. What happens if you forget the KEY? The write is silently dropped by
the hardware: the GPIO registers read back as all zeros, the LED does not
budge, and no error is reported anywhere. This is one of the details where
the CW32 family differs most from STM32 (STM32's RCC needs no key).

### Segment 3: Which reset leftovers into_output clears

`into_output(Level::Low)` performs four steps in order (excerpt from
cw32l012-hal `src/gpio.rs`; for the direction convention see manual
§9.6.1 — **DIR=0 is output, the opposite of STM32**):

```rust,ignore
pub fn into_output(self, initial: Level) -> Output<'p> {
    let m = self.mask();
    match initial {
        Level::High => self.regs.bsrr.write(m),
        Level::Low => self.regs.bsrr.write(m << 16),
    }
    self.clear_extras();
    self.regs.dir.modify(|v| v & !m); // 0 = output
    Output { pin: self }
}
```

The initial level is written atomically through BSRR first and the direction
is switched afterwards, so no glitch can pop out at the switching instant.
`clear_extras()` then zeroes four kinds of "reset leftovers" in one pass:

```rust,ignore
fn clear_extras(&self) {
    let m = self.mask();
    self.regs.opendrain.modify(|v| v & !m);
    self.regs.pur.modify(|v| v & !m);
    self.regs.pdr.modify(|v| v & !m);
    self.regs.analog.modify(|v| v & !m);
    self.set_af(0);
}
```

Open-drain, pull-up, pull-down, and resetting the alternate function to zero
are all easy to grasp; the crucial one is the **ANALOG bit**: after reset,
PC13 on the CW32L012 is in **analog mode** (`GPIOx_ANALOG.PIN13 = 1`) — a
pitfall discovered on real hardware. In analog mode the digital output
driver is disconnected; if the HAL only wrote DIR without clearing ANALOG,
every register would look right and the LED still would not blink.
`into_output` clears that bit along the way, which is what lets the example
"light an LED in one line". Conversely, the ADC/OPA pins of Chapter 11
*want* analog mode: `Pin::new` clears it, so analog uses must explicitly
call `into_analog()` to set it back — the HAL documentation records a
measured case where PB00, left out of analog mode, had its op-amp output
clamped low by the digital driver.

### Segment 4: toggle() and the TOG register

```rust,ignore
led.toggle();
```

The flip is not the three-step "read ODR → XOR → write back"; you write a
bit mask to the `TOG` register (@0x60) and the hardware completes the flip
in one operation. Atomicity means an interrupt arriving at any moment cannot
tear this level change — you will meet it again in the timer ISR of
Chapter 6.

### Segment 5: Delay's calibration reference

```rust,ignore
let mut delay = Delay::new(&clocks);
delay.delay_ms(500);
```

`Delay` busy-waits on the Cortex-M0+ SysTick with the clock source fixed to
HCLK, and what HCLK actually is comes from the `Clocks` snapshot **passed in
at construction**. `Delay::new` precomputes the two coefficients "ticks per
microsecond / ticks per millisecond", avoiding the hardware division the M0+
lacks (`u64` division runs through a software routine; in a debug build a
single division costs on the order of a thousand cycles). Note the
corollary: **if you change the main frequency after `Delay::new`, your
delays are wrong** — it does not follow the clock. Settle the frequency
first, then build the `Delay`.

## 4.4 Verification on Hardware

**Status: verified via the same-source example.** This chapter's example is
line-for-line identical in source to `examples/blink_pc13.rs` in the
cw32l012-hal repository (only rewritten per edition 2024 with
`#[unsafe(no_mangle)]` and adjusted comments). The original example is
recorded in the "verified on hardware" table of cw32l012-hal's `TESTING.md`
(test memo, measured 2026-10-17 through 10-19):

| Example | What was verified | Key readings |
|------|----------|----------|
| blink_pc13 | GPIO key gating → ANALOG cleared → BSRR/TOG | PC13 500 ms square wave |

That is: the complete chain — reset-default clocks, AHBEN gating, ANALOG
clearing, TOG toggling — was confirmed on a real board in the form of a
500 ms square wave, with register read-backs matching the manual. Delay
accuracy has its own `clk_check` entry: `delay_ms(1000)` is exactly 1000
ticks; `delay_us` error is +0.5% to +1.1% (at the 4 MHz setting).

## 4.5 Common Pitfalls

- **Forgetting clock gating**: writing GPIO registers by hand without
  setting the AHBEN bit — every write is silently dropped. The HAL's
  `Pin::new` enables it automatically; this is where you most easily trip
  when bypassing the HAL.
- **Writing the gating without KEY=0x5A5A**: likewise silently dropped, with
  no error flag to check anywhere.
- **PC13 resets with ANALOG=1**: without clearing this bit, configuring DIR
  produces no output — for any "registers are all correct but there is no
  waveform on the pin" mystery, check ANALOG first.
- **DIR's direction semantics are opposite to STM32**: on CW32, 0 = output
  and 1 = input (§9.6.1); code migrated from STM32 that tweaks DIR on old
  instinct gets the opposite effect.
- **Delay::new before changing the frequency**: delays are based on the HCLK
  snapshot taken at construction; after a frequency change you must rebuild
  `Delay`, or 500 ms may not be 500 ms.
