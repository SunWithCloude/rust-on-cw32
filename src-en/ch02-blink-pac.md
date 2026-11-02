# Chapter 2 Blink from Scratch: the PAC and Registers

In this chapter you will learn:

- the "essential four" of a `no_std` bare-metal project: Cargo.toml, memory.x, build.rs, and the linking conventions;
- the role of the PAC (Peripheral Access Crate): turning "read and write an address" into "typed register objects";
- the L012's peripheral clock gating and the `KEY = 0x5A5A` write protection;
- the complete register path for lighting PC13: clock gating → out of analog mode → set direction → toggle a bit;
- what `volatile` actually defends against — which things the compiler would otherwise "optimize" away for you;
- engineering it: folding the three pieces of boilerplate beyond panic (the delay, the LED sequence) into the crate library `src/lib.rs`, and how the delay upgrades from "counting loop iterations" to SysTick hardware timing.

This chapter uses no HAL and no wrappers of any kind: one line of register code is one bus write. Once you understand this chapter, every later HAL/async wrapper is transparent to you.

## The Bare-Metal Essentials

Chapter 1 already built the `examples/pac` crate; now open it and look at everything inside. Besides Cargo.toml, a bare-metal project that runs on a Cortex-M0+ needs three more things.

**First, the dependency declarations.** The key parts of `examples/pac/Cargo.toml`:

```toml
[package]
name = "cw32-book-pac"
edition = "2024"
publish = false

[dependencies]
cw32l012-pac = { path = "../../../cw32l012-pac", features = ["rt"] }
```

`features = ["rt"]` makes the PAC additionally generate two link-time artifacts: the `__INTERRUPTS` interrupt vector table and the `device.x` interrupt symbol list (see the PAC's `docs/rt-smoke.md` for details). Without `rt`, the PAC is just "a pile of address constants"; the linked firmware has no vector table, and after reset the CPU cannot fetch its first instruction.

**Second, memory.x.** It tells the linker how much memory this chip has, and where:

```text
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 64K
  RAM   : ORIGIN = 0x20000000, LENGTH = 8K
}
```

The CW32L012 has 64 KB of Flash and 8 KB of SRAM; both numbers come straight from the datasheet's capacity specifications. cortex-m-rt's `link.x` reads this file to lay out the vector table and the placement of `.data`/`.bss`.

**Third, how the link arguments get delivered.** The full text of `examples/pac/build.rs` (comments removed):

```rust
use std::env;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=memory.x");
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rustc-link-search=native={}", manifest.display());
    println!("cargo::rustc-link-arg=-Tlink.x");
}
```

It adds the directory containing `memory.x` to the link search path and attaches `-Tlink.x` (cortex-m-rt's linker script) to every binary. Why not put `rustflags` in `.cargo/config.toml`? Because `.cargo/config.toml` is only read by cargo when the build runs **with this crate's directory as the current directory** — and this book's acceptance command (Chapter 1) runs from the workspace root, where a config file in a subdirectory is never looked at. Link arguments delivered by `build.rs` always take effect.

The `.cargo/config.toml` file itself is kept (a few comment lines only, noting that the linker script is build.rs's job) as the future home for directory-local settings such as a `runner`.

## Peripherals Are Addresses; the PAC Makes Them Types

The memory map table in the first chapter of the chip manual (Table 2-1) tells you: the SYSCTRL register block starts at `0x4000_4000`, and GPIOC's at `0x4800_0200`. The essence of bare-metal programming is reading and writing these addresses.

What the PAC does is turn these addresses into **typed objects**:

- one `struct` per peripheral (such as `Sysctrl`, `Gpio`), with fields laid out by register offsets; `repr(C)` guarantees the memory layout matches the manual word for word;
- one access type per register: `Rw<u32>` is readable and writable, `Ro<u32>` read-only, `Wo<u32>` write-only — each exposing only its legal methods (`read`/`write`/`modify`);
- the bit fields and values of every register are constants (such as `sysctrl::AHBEN_GPIOC`); the API names are transcribed directly from the manual.

The entry point is the peripheral singleton:

```rust
let p = Peripherals::take().unwrap();
```

`Peripherals::take()` can only succeed once; a second call returns `None` — Rust's ownership mechanism guarantees that "one peripheral cannot be configured at will by two pieces of code at the same time". Behind fields like `p.sysctrl` and `p.gpioc` sit exactly the two addresses above; `p.gpioc.analog` is an `Rw<u32>`, and calling `read()`/`modify()` on it expands to one volatile memory-access instruction.

> One discipline: **the firmware must genuinely use the PAC**. Even a single line like `Peripherals::take()` is enough to pull the PAC's rlib onto the linker command line; if the program references no PAC symbol at all, the linker reports `The interrupt vectors are missing` — the vector table is not wrong, it simply was never linked in (a pitfall recorded in the PAC's `docs/rt-smoke.md`; this chapter's example cannot hit it, because `take()` itself references the PAC).

## Step 1: Clock Gating and KEY Write Protection

After reset on the CW32L012, **the clocks of all peripherals except SysTick and SRAM are off** (manual §2). The manual spells out the consequence of writing to a peripheral whose configuration clock is not enabled: the write fails, **no HardFault is raised**, and data read back is unusable — the error happens silently, which makes this the easiest trap of this kind of chip. So step 1 of lighting an LED is always: turn on GPIOC's clock.

GPIO clocks go through the AHB gating register `SYSCTRL_AHBEN` (manual §4.7.11, offset 0x30, Table 4-9): bits 4/5/6/7 are GPIOA/B/C/F. To light PC13, set bit 6:

```rust
p.sysctrl.ahben.modify(|v| v | sc::AHBEN_GPIOC);
```

— **written this way, it does nothing**. `AHBEN` carries KEY write protection: the upper 16 bits are a KEY field, write-only, **and a write to this register takes effect only when the upper 16 bits of the written value are `0x5A5A`** (manual §4.7.11 bit-table wording: "writes to this register are valid only when KEY is 0x5A5A"; the flow description in §4.5 likewise stresses "the upper 16 bits of the written data must be 0x5A5A, otherwise the write cannot happen"). The PAC turned this key into a constant:

```rust
/// Write-protection key: … the upper 16 bits must be 0x5A5A, otherwise the write is ignored
pub const KEY: u32 = 0x5A5A << 16;
```

(from the `sysctrl` module of the PAC's `src/lib.rs`.)

The trouble is that KEY is a write-only field and **always reads back as 0** (the comment in the HAL's `rcc.rs` records the same fact). The standard "change one bit" sequence therefore becomes three stages: read the register, strip the upper-16-bit KEY from what was read (it reads as 0 anyway; `& 0xFFFF` expresses the intent), then write back once with KEY restored and the wanted bit ORed in. This is exactly the implementation of `set_bits` in `cw32l012-hal`'s `rcc.rs`, and this chapter's example adopts the same steps verbatim:

```rust
let v = p.sysctrl.ahben.read() & 0xFFFF;        // lower 16 bits are the gating state
p.sysctrl
    .ahben
    .write(sc::KEY | v | sc::AHBEN_GPIOC);      // KEY + current state + GPIOC (bit6)
```

Note that this is not `modify()` — `modify` is "read, process in a closure, write back"; if the closure does not know to restore KEY, the written value has 0 in the upper 16 bits and the hardware discards the entire write. KEY-protected registers (CR0/CR1/CR2/AHBEN/APBEN1/2, the reset registers, and so on) must all take the "restore KEY" path.

## Step 2: Bringing PC13 Back from Analog Mode

Every pin of the L012 comes out of reset in **analog mode**: the corresponding bit of `GPIOx_ANALOG` is 1. Manual §9.6.7 gives the reset value — for GPIOC it is `0x0000_E000`, meaning PC13/14/15 all default to analog mode. In analog mode "the port's digital function is disabled" (manual §9.3): digital input and output are cut off. Without clearing this bit, writing DIR or TOG later is futile.

In the PAC, GPIOC's register block is a single `Gpio` struct, with the `analog` field at offset 0x1C. Clear the PIN13 bit:

```rust
p.gpioc.analog.modify(|v| v & !PIN13);
```

An ordinary readable/writable register — `modify()` is exactly right here (read, clear the bit, write back).

## Step 3: Setting the Direction to Output

The direction register `GPIOx_DIR` (manual §9.6.1) follows a convention **inverted** relative to most MCUs (including STM32):

- `DIR.PINy = 1` means **input**;
- `DIR.PINy = 0` means **output**.

The PAC turned this pair of values into the constants `gpio::DIR_INPUT = 1` / `gpio::DIR_OUTPUT = 0`. GPIOC's DIR reset value is likewise `0x0000_E000` (§9.6.1) — PC13 comes out of reset as an input. To make it an output, clear the bit:

```rust
p.gpioc.dir.modify(|v| v & !PIN13);
```

The output mode (push-pull/open-drain) is controlled by `GPIOx_OPENDRAIN`, which resets to push-pull; this example does not touch it.

## Step 4: Toggling via TOG + a Software Delay

For lighting and extinguishing, the most economical is the bit-toggle register `GPIOx_TOG` (manual §9.6.17): **writing 1 to a bit atomically toggles the corresponding `ODR` output bit in hardware** (manual §9.3). It is a write-only register (a `Wo<u32>` in the PAC), so there is no read-back interference and none of the read-modify-write three-step race:

```rust
p.gpioc.tog.write(PIN13);   // toggles PC13 once
```

`PIN13` is just `1 << 13` — GPIO register bit fields are laid out by pin number PINy throughout.

Some time is needed between toggles, so insert a software delay. **There is a trap in how the delay is written**: a plain `for _ in 0..n {}` has no side effects as far as LLVM can see in a release build, so it deletes the loop entirely — the firmware "delays" for zero cycles, and the LED appears not to blink, or to blink frantically. The defense is to make the loop body invisible to the optimizer: either volatile accesses (the compiler may not delete volatile operations) or `core::hint::black_box`. This example counts with volatile, which happens to be the same primitive as the register accesses:

```rust
fn delay(n: u32) {
    let mut left = n;
    unsafe {
        let p = core::ptr::addr_of_mut!(left);
        while p.read_volatile() > 0 {
            p.write_volatile(p.read_volatile() - 1);
        }
    }
}
```

`delay(500_000)` at the reset-default HCLK = 4 MHz (HSI, §4.3.4) takes on the order of 0.5–1 s — not calibrated, and it does not need to be: LED blinking only asks for a "rhythm visible to the eye". For a millisecond-calibrated delay, the engineering section at the end of this chapter builds `board::delay_ms` on the SysTick hardware counter, and Chapter 4's HAL is its full-grown form.

## The Full Example

`examples/pac/examples/ch02_blink.rs`:

```rust
//! Chapter 2 example: PAC raw-register blink — GPIOC clock gating (KEY write
//! protection) → PC13 out of analog mode → TOG toggle + software delay.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch02_blink -p cw32-book-pac
//! Status: compiles, not yet run on hardware; the register path is the same
//! source as the hardware-verified cw32l012-hal blink_pc13 example
//! (TESTING.md: KEY gating → ANALOG clear → TOG, 500 ms square wave on PC13).

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use cw32l012_pac::{Peripherals, sysctrl as sc};

/// Bit mask of PC13 in the port registers (GPIO bit fields are all numbered
/// by PINy: PIN13 = bit13)
const PIN13: u32 = 1 << 13;

/// Rough software delay (on the order of 0.5–1 s at the reset-default
/// HCLK = 4 MHz; not calibrated). Counting through volatile keeps the idle
/// loop from being optimized away — without volatile, a release build
/// deletes the loop outright.
fn delay(n: u32) {
    let mut left = n;
    // SAFETY: volatile access to a stack local through a raw pointer,
    // confined to this function; single core, no contention
    unsafe {
        let p = core::ptr::addr_of_mut!(left);
        while p.read_volatile() > 0 {
            p.write_volatile(p.read_volatile() - 1);
        }
    }
}

#[entry]
fn main() -> ! {
    // Peripheral singleton: Peripherals::take succeeds only once (None means already taken)
    let p = Peripherals::take().unwrap();

    // ---- Step 1: turn on GPIOC's clock (SYSCTRL_AHBEN, manual §4.7.11) ----
    // KEY write protection: the upper 16 bits of AHBEN must be written as
    // 0x5A5A or the whole write is ignored (PAC sysctrl::KEY = 0x5A5A << 16);
    // KEY always reads back as 0, so read the lower 16 bits before changing
    // anything and restore KEY verbatim when writing back (the same steps as
    // cw32l012-hal rcc::set_bits).
    let v = p.sysctrl.ahben.read() & 0xFFFF;
    p.sysctrl
        .ahben
        .write(sc::KEY | v | sc::AHBEN_GPIOC); // AHBEN.GPIOC = bit6 (§4.7.11)

    // ---- Step 2: bring PC13 back from analog mode (GPIOC_ANALOG, manual §9.6.7) ----
    // Reset default GPIOx_ANALOG = 0xE000 (GPIOC), PIN13 = 1 (analog
    // function), digital input/output cut off; must be cleared before
    // lighting (§9.4.1 step 1).
    p.gpioc.analog.modify(|v| v & !PIN13);

    // ---- Step 3: set the direction to output (GPIOC_DIR, manual §9.6.1) ----
    // Inverted relative to most MCUs: DIR.PINy = 1 is input, 0 is output
    // (PAC gpio::DIR_OUTPUT = 0); GPIOC resets to 0xE000, leaving PC13 an
    // input, so clear the bit (§9.4.1 step 2).
    p.gpioc.dir.modify(|v| v & !PIN13);

    loop {
        // ---- Step 4: toggle the output (GPIOC_TOG, manual §9.6.17) ----
        // TOG is write-only: bits written 1 have their ODR bits toggled
        // atomically by hardware (§9.3); no read-modify-write needed.
        p.gpioc.tog.write(PIN13);
        delay(500_000);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
```

`#![no_std]` excludes the standard library, `#![no_main]` hands over the right to define `main`, and `#[entry]` (cortex-m-rt) marks the function as the reset entry point and requires the signature `fn() -> !` (the entry never returns). The panic handler is the smallest form that satisfies the compiler — this chapter has no logging channel yet, so a panic can only spin in place (from Chapter 3 on, panic learns to talk).

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch02_blink -p cw32-book-pac
```

The artifact lands at `target/thumbv6m-none-eabi/release/examples/ch02_blink`. Flash it to the board and run (`probe-rs download` + `probe-rs reset`; command templates in Appendix D), and the on-board PC13 LED should alternate bright and dark with a period of about one second.

## On-Hardware Verification

**This example itself: compiles, not yet run on hardware** (thumbv6m-none-eabi release build passes; the `__INTERRUPTS` vector table lands correctly).

The example's register write sequence is, however, **verified via the same-source example**: cw32l012-hal's `blink_pc13` example was verified on that same board — TESTING.md (2026-10-17) records its verified content as "GPIO KEY gating → ANALOG clear → BSRR/TOG", with the key reading "500 ms square wave on PC13" (source: cw32l012-hal `TESTING.md`). The only differences in this example: it bypasses the HAL wrappers, and the delay is not calibrated (a rhythm of about 1 s instead of 500 ms).

Three things worth checking on the first board run:

1. If the LED does not blink, first suspect a missed gating/ANALOG step (errors are silent; see §2's "no HardFault is raised");
2. If brightness is normal but it never blinks, TOG is running while the delay was optimized away — check that the build is release and the delay uses volatile;
3. Break at `main` with probe-rs and single-step, watching how the lower 16 bits of `p.sysctrl.ahben.read()` change before and after the `ahben` write.

## Engineering It: From Example to Library

Look back at ch02_blink: it contains three things that the next example would copy over unchanged — the software delay, the three-step register sequence for lighting the LED, and the panic handler. Boilerplate is teaching the first time it appears, and debt the second — every copy can be miscopied, and a fix can miss a copy. The engineering answer is to give the crate a **lib target**: create `src/lib.rs` under `examples/pac/`, and cargo automatically makes it a dependency of every example — **no** dev-dependencies changes needed. This "library of your own" looks like this:

```rust
//! # cw32-book-pac
//!
//! Shared convenience layer for the Chapter 1–3 examples. The Chapter 2
//! section "Engineering It: From Example to Library" records how this lib
//! came to be: boilerplate that "every example copies once" — the panic
//! handler, the software delay, the LED register sequence — belongs in the
//! library from the second example onward.
//!
//! - [`board`]: SysTick busy-wait delays (`delay_ms`/`delay_us`) and the
//!   PC13 on-board LED ([`board::Led`]), register path line-for-line the
//!   same source as `ch02_blink.rs`.
//! - The panic handler is **not in the library**: Chapter 2's teaching
//!   thread is precisely "why a no_std firmware must provide its own", so
//!   `#[panic_handler]` still ships with each example (switched to
//!   panic-probe from Chapter 3 onward; the `examples/hal` and
//!   `examples/embassy` crates fold the panic handler into their lib.rs —
//!   see the corresponding chapters).
//!
//! The `board` module exists only on ARM bare-metal targets (cortex-m
//! platform code does not compile on the host); on a host build this crate
//! is an empty shell.

#![no_std]

#[cfg(all(target_arch = "arm", target_os = "none"))]
pub mod board;
```

The library's entire content is one `board` module (`src/board.rs`) with two small wrappers:

```rust
//! Small board-level wrappers: SysTick busy-wait delay + PC13 on-board LED.
//!
//! Why this went into the lib: see Chapter 2's "Engineering It" section —
//! delay and blink are boilerplate every example repeats. The register path
//! is line-for-line the same source as `examples/ch02_blink.rs`:
//! clock gating (KEY write protection) → out of analog mode → direction
//! output → TOG/BSRR toggle.

use cortex_m::peripheral::syst::SystClkSource;
use cortex_m::peripheral::Peripherals as CorePeripherals;
use cw32l012_pac::{Peripherals, sysctrl as sc, Gpio};

/// Bit mask of PC13 in the port registers (GPIO bit fields are all numbered
/// by PINy: PIN13 = bit13)
pub const PIN13: u32 = 1 << 13;

/// Reset-default clock: HCLK = 4 MHz (HSI, manual §4.3.4 / §4.7.4).
/// If the firmware changes the clock, update this constant in step —
/// delay lengths will drift proportionally.
pub const HCLK_HZ: u32 = 4_000_000;

// ================= SysTick delay =================

// SysTick is a **core** peripheral of ARMv6-M (register block in the system
// control space at 0xE000_E010, ARMv6-M Architecture Reference Manual DDI
// 0419 §B3.3), not one of the CW32L012's chip peripherals, so the
// hand-written PAC (cw32l012-pac) does not model it — the delay can only be
// reached through the cortex-m crate's `SYST` type (that crate is already
// in the workspace dependency tree, pulled in via embassy-executor's
// cortex-m platform layer; examples/pac declares 0.7 explicitly to align
// versions).
//
// How it works (three things): `RVR` (LOAD) holds "how many ticks to
// count"; writing anything to `CVR` (VAL) clears and restarts the count;
// and in `CSR` (CTRL), CLKSOURCE=1 selects HCLK, ENABLE=1 starts counting,
// **COUNTFLAG is set when the count reaches 0 and reading CSR clears it
// automatically** — polling it gives you "one timed interval". This module
// fires one single countdown per millisecond / per microsecond chunk.

/// SysTick ticks per microsecond (CLKSOURCE=1: count HCLK directly)
const TICKS_PER_US: u32 = HCLK_HZ / 1_000_000;
/// How many microseconds one countdown of the 24-bit counter spans at most
/// (2^24 / TICKS_PER_US ≈ 4.19 s)
const MAX_US: u32 = (1 << 24) / TICKS_PER_US;

/// Microsecond busy-wait delay (one SysTick countdown + COUNTFLAG polling).
/// Spans beyond a single range (about 4.19 s @4 MHz) are split and continued
/// automatically.
pub fn delay_us(us: u32) {
    let mut left = us;
    while left > 0 {
        let chunk = if left > MAX_US { MAX_US } else { left };
        one_shot(chunk * TICKS_PER_US);
        left -= chunk;
    }
}

/// Millisecond busy-wait delay: one single countdown per millisecond. The
/// fixed cost of reloading between shots is only a few dozen cycles, far
/// below 1 ms — invisible to a "rhythm visible to the eye".
pub fn delay_ms(ms: u32) {
    for _ in 0..ms {
        delay_us(1000);
    }
}

/// Load `ticks` ticks and return only when the count reaches 0 (blocking).
fn one_shot(ticks: u32) {
    // SAFETY: only borrows the core-peripheral handle to read and write the
    // three SysTick registers; touches no chip-peripheral configuration.
    // Single-core bare metal, called before interrupts are enabled: no races.
    let mut cp = unsafe { CorePeripherals::steal() };
    let syst = &mut cp.SYST;
    syst.set_clock_source(SystClkSource::Core); // CLKSOURCE=1: count HCLK
    syst.set_reload(ticks - 1); // RVR: count down from ticks-1 to 0
    syst.clear_current(); // writing anything to CVR clears the count (COUNTFLAG goes too)
    syst.enable_counter(); // ENABLE=1
    while !syst.has_wrapped() {
        core::hint::spin_loop(); // reaching 0 sets COUNTFLAG; reading CSR clears it automatically
    }
    syst.disable_counter(); // reprogrammed on the next call; idle the rest of the time
}

// ================= PC13 LED =================

/// Wrapper for the PC13 on-board LED.
///
/// Construction performs Chapter 2's three-step register sequence; toggle
/// and level-setting go through the write-only TOG/BSRR registers. The
/// gating step is a read-modify-write that "reads the lower 16 bits and
/// restores KEY on write-back" (the same order as cw32l012-hal
/// `rcc::set_bits`), which **preserves** the other port gate bits already
/// open in the same firmware.
pub struct Led {
    gpioc: &'static Gpio,
}

impl Led {
    /// Build the PC13 LED from the already-taken peripheral singleton.
    ///
    /// The internal three steps are line-for-line the same source as steps
    /// 1–3 of `ch02_blink.rs`: GPIOC gating (AHBEN, KEY write protection,
    /// manual §4.7.11) → ANALOG clear (§9.6.7) → DIR output (§9.6.1; on the
    /// L012 only DIR=0 is output).
    pub fn new(p: &Peripherals) -> Self {
        // KEY always reads back as 0: read the lower 16 bits first and
        // restore KEY verbatim on write-back, or the hardware silently
        // drops the whole write
        let v = p.sysctrl.ahben.read() & 0xFFFF;
        p.sysctrl.ahben.write(sc::KEY | v | sc::AHBEN_GPIOC);
        Self::from_gpioc(p.gpioc)
    }

    /// Constructor for when AHBEN is already configured on its own (e.g.
    /// combined with other port bits in the same write): performs only the
    /// analog-mode and direction steps.
    pub fn from_gpioc(gpioc: &'static Gpio) -> Self {
        // Reset default GPIOx_ANALOG = 0xE000 (GPIOC): PIN13=1 analog mode,
        // digital input/output cut off; must be cleared (manual §9.6.7 / §9.4.1 step 1)
        gpioc.analog.modify(|v| v & !PIN13);
        // DIR.PIN13 cleared = output (§9.6.1: inverted vs. most MCUs, 1 is input)
        gpioc.dir.modify(|v| v & !PIN13);
        Led { gpioc }
    }

    /// Toggle the level: write the TOG bit mask; hardware atomically
    /// toggles ODR (manual §9.6.17).
    pub fn toggle(&mut self) {
        self.gpioc.tog.write(PIN13);
    }

    /// Set the level: true = drive high (BSRR set field), false = drive low
    /// (BSRR clear field, i.e. the bit mask shifted left by 16).
    ///
    /// Note: **which level means "lit" depends on how the board wires the
    /// LED** (sourcing or sinking current); this wrapper makes no promise —
    /// Chapter 2's examples always use `toggle()`, which covers both wirings.
    pub fn set(&mut self, high: bool) {
        let bits = if high { PIN13 } else { PIN13 << 16 };
        self.gpioc.bsrr.write(bits);
    }

    /// Drive high (whether that lights the LED depends on the board wiring;
    /// see [`Led::set`])
    pub fn on(&mut self) {
        self.set(true);
    }

    /// Drive low (whether that lights the LED depends on the board wiring;
    /// see [`Led::set`])
    pub fn off(&mut self) {
        self.set(false);
    }
}
```

Three design decisions, one reason each:

**Why the delay switched to SysTick.** `board::delay_ms` no longer counts loop iterations; it drives SysTick — a 24-bit down-counter. SysTick is not on the CW32L012's list of chip peripherals; it is a **core peripheral of the Cortex-M0+** (register block starting at 0xE000_E010 in the system control space, ARMv6-M Architecture Reference Manual DDI 0419 §B3.3), so the hand-written PAC does not model it — which is also why this chapter's example could only "count loops" until now: pure-PAC firmware has no typed access to SysTick. The fix is to bring in the `cortex-m` crate explicitly (it has been in the workspace dependency tree all along — embassy-executor's cortex-m platform layer depends on it) and use its `SYST` type to reach the three registers: `RVR` (LOAD) holds "how many ticks to count"; writing anything to `CVR` (VAL) clears and restarts; and in `CSR` (CTRL), `CLKSOURCE=1` selects HCLK and `ENABLE=1` starts counting, with **COUNTFLAG set when the count reaches 0 and cleared automatically by reading CSR** — polling it is "one timed interval". `delay_us` fires single shots in segments within the 24-bit range (2^24 / 4 ≈ 4.19 s @4 MHz), and `delay_ms` fires one shot per millisecond. The time base moves from "instruction cycles" to "hardware ticks": at HCLK = 4 MHz, 1 ms is exactly 4000 ticks (reloading between shots costs only a few dozen cycles), no longer an order-of-magnitude estimate of "about 0.5–1 s". If the firmware changes the clock, update `board::HCLK_HZ` in step.

**Which lines `Led` absorbed.** `Led::new` is line-for-line the same source as steps 1–3 of ch02_blink.rs: gating (keeping the "read the lower 16 bits + restore KEY" pattern, so it does not clobber the other port enable bits already open in the same firmware), the ANALOG clear, and DIR output; `toggle()` goes through TOG, and `set()/on()/off()` through BSRR. `on()/off()` promise only a level direction, not "lit" — LED polarity depends on the board wiring (sourcing or sinking current), and the examples always use `toggle()`, which covers both.

**Why the panic handler deliberately stays out of the library.** It is one of this chapter's teaching protagonists (why a no_std firmware must provide its own, and why the compiler forces you to write one); hiding it in the lib would ruin that lesson. In Parts 2 and 3 (`examples/hal`, `examples/embassy`), the panic handler is folded into each lib.rs — the examples inherit it with a single `use cw32_book_hal as _;` / `use cw32_book_embassy as _;`, the exact same mechanism as Chapter 3's `use panic_probe as _;`.

With `board` in place, the same blink example looks like this (`examples/pac/examples/ch02_blink_lib.rs`):

```rust
//! Chapter 2 example 2: the engineered form of blink — register boilerplate
//! moved into the crate library (lib.rs).
//!
//! Hardware behavior is identical to ch02_blink.rs, but the three steps
//! (gating / analog-mode exit / direction) went into `board::Led::new`, and
//! the delay into `board::delay_ms` (SysTick busy-wait, millisecond-calibrated
//! at the reset-default HCLK = 4 MHz). For the raw-register teaching version
//! see ch02_blink.rs — reading the two side by side is the "example to
//! library" evolution itself.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch02_blink_lib -p cw32-book-pac
//! Status: compiles, not yet run on hardware; the register sequence is
//! verbatim the same source as ch02_blink.rs, which in turn is the same
//! source as the hardware-verified cw32l012-hal blink_pc13 example
//! (TESTING.md: 500 ms square wave on PC13).

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use cw32_book_pac::board::{Led, delay_ms};
use cw32l012_pac::Peripherals;

#[entry]
fn main() -> ! {
    // The example still takes the peripheral singleton — "who owns the
    // peripherals" is not hidden inside the library
    let p = Peripherals::take().unwrap();

    // Construction performs ch02's three steps: gating (KEY write
    // protection) → out of analog mode → direction output
    let mut led = Led::new(&p);

    loop {
        led.toggle(); // TOG bit toggle (the same register write as ch02_blink)
        delay_ms(500); // SysTick busy-wait 500 ms (one countdown per millisecond)
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
```

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch02_blink_lib -p cw32-book-pac
```

`main` is down to three things: take the peripheral singleton, construct the LED, toggle + delay. Not one register operation was dropped — it just moved into the library, and from now on it is written once. That is what "abstraction" means on bare metal: not hiding the registers away, but putting the sequence you have confirmed correct into the one place you edit. (Answering a common question in passing: why isn't `Peripherals::take()` also hidden inside `Led::new`? — The peripheral singleton can be taken only once, and whoever takes it holds it; hiding it in the library would mean the second caller that needs peripherals gets nothing. Ownership stays in the example's hands; the library only borrows.)

**ch02_blink_lib verification status: compiles, not yet run on hardware**; its register sequence is verbatim the same source as ch02_blink.rs, which is "verified via the same-source example" (hardware-tested cw32l012-hal blink_pc13, above; source: cw32l012-hal `TESTING.md`). The delay mechanism (SysTick counting HCLK ticks) shares one hardware path with Chapter 4's HAL `Delay` — the `clk_check` entry measured `delay_ms(1000)` at exactly 1000 ticks (TESTING.md), so the mechanism's accuracy has measured corroboration.

## Common Pitfalls

- **Forgot to turn on GPIOC's clock**: every later register write fails silently (no HardFault) and the LED never moves. The first step with any peripheral is checking `SYSCTRL_AHBEN`/`APBEN`.
- **Forgot to restore KEY**: `modify(|v| v | sc::AHBEN_GPIOC)` has no effect on a KEY-protected register — the upper 16 bits read as 0 and the whole written value is discarded. KEY registers require "read the lower 16 bits + restore `sc::KEY` on write-back".
- **DIR direction remembered backwards**: on the L012, `DIR=1` is input. With the wrong direction the pin neither drives nor errors; reads return whatever the input buffer holds.
- **ANALOG not cleared**: PC13 resets into analog mode with digital output cut off; the symptom is the same "silent no-op" as a forgotten clock.
- **The delay loop optimized away**: in release builds an idle loop vanishes entirely; keep it alive with volatile or `black_box` (`board::delay_ms` switched to SysTick hardware counting and no longer has this problem; from Chapter 4 it is the HAL's `Delay`).
- **Deleted `Peripherals::take()`**: if the firmware no longer references any PAC symbol, the link fails with `The interrupt vectors are missing`.
