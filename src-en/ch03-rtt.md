# Chapter 3 Making the Chip Talk: RTT Logging

In this chapter you will learn:

- what RTT (Real-Time Transfer) is, and why logging does not need a UART;
- the defmt logging framework: format strings into the ELF, decoding on the host;
- the trio `defmt` + `defmt-rtt` + `panic-probe`, and the critical-section implementation you must provide yourself on bare-metal M0+;
- what `_defmt_timestamp` is for; defmt 1.x provides timestamps through the `timestamp!` macro;
- flashing, running, and collecting logs in one command with `probe-rs run`.

Chapter 2's LED can only say "blinking or not"; when something goes wrong, all you can do is stare at it. From this chapter on, the chip can report to your computer on its own — without occupying any UART pin, everything over the one debug cable.

## What RTT Is

RTT (Real-Time Transfer) is a debug-channel protocol designed by SEGGER, and the idea is direct: firmware and debugger agree on a **ring buffer in SRAM** (the control block `_SEGGER_RTT`); the firmware writes into it, and the debugger reads it out over SWD. On the CPU side it is just a few memory writes, done in microseconds, non-blocking — that is the "real-time" in the name.

Compared with the old route of logging over UART, RTT's advantages on this board are very concrete:

- **No pins used**. UART1 takes PA02/PA03; RTT rides the debug cable — not one wire beyond the four SWD lines.
- **No baud rate to match**. Both UART ends must agree on the same baud rate; 2% off means garbled text. RTT is a memory read; there is no rate negotiation.
- **Fast**. The SWD clock starts at several MHz, far above UART's 115200.

The price: RTT needs a "live" debugger reading online — unplug the debugger and the log is gone. Use RTT for debug-time logging and UART for production-time external communication; the two do not conflict.

## defmt: Logs That Save Space Too

RTT answers "how the bytes get out"; [defmt](https://defmt.ferrous-systems.com/) answers "which bytes get sent". It compiles logs like `info!("heartbeat #{}", n)` into compact binary frames: **the format string never enters the firmware** — it is interned into the `.defmt` metadata section of the ELF, the firmware carries only a 2-byte index, and numeric parameters are written into the frame in a compact encoding. The host side (probe-rs) reads the metadata table from the ELF and turns the frames back into human-readable text.

For a chip with 64 KB of Flash, this design is not a luxury but a necessity — the few dozen bytes saved per format string, multiplied by the number of log lines, quickly add up to a whole functional module.

defmt's log macros come in levels: `trace!`/`debug!`/`info!`/`warn!`/`error!`. Level filtering happens **at compile time** (controlled by the `DEFMT_LOG` environment variable); a filtered-out log costs not a single byte.

## The Trio: defmt-rtt, panic-probe, and a critical-section Implementation

The development dependencies of `examples/pac/Cargo.toml` (enabled only for the ARM bare-metal target):

```toml
[target.'cfg(all(target_arch = "arm", target_os = "none"))'.dev-dependencies]
cortex-m-rt = { version = "0.7", features = ["device"] }
critical-section = { version = "1", default-features = false, features = ["restore-state-u8"] }
defmt = "1"
defmt-rtt = "0.4"
panic-probe = { version = "0.3", features = ["print-defmt"] }
```

Each of the three crates has its own job:

- **`defmt-rtt`**: registers defmt's global logger and writes defmt frames into the RTT upstream channel. It places the `_SEGGER_RTT` control block in SRAM (a 1024-byte ring buffer, adjustable through the `DEFMT_RTT_BUFFER_SIZE` environment variable), and the debugger finds it by the control block symbol. Note that it **cannot coexist** with `rtt-target` (the plain-text RTT library) — there can only be one control block, and defmt-rtt forbids the mix at the source level.
- **`panic-probe`** (the `print-defmt` feature): provides the panic handler. On panic it prints the panic location through defmt and then halts, replacing Chapter 2's handler that only spun in place.
- **`critical-section`**: a convention for "who provides the critical section". defmt-rtt's logging lock is built on `critical_section::acquire()`, and **bare-metal M0+ has no ready-made implementation** — when nobody provides one, the link fails outright:

  ```text
  rust-lld: error: undefined symbol: _critical_section_1_0_acquire
  ```

  The fix is to `set_impl!` one in the firmware yourself. The standard single-core recipe is "read PRIMASK → `cpsid i` to mask interrupts", and on exit restore with `cpsie i` only if interrupts were originally enabled. The implementation in the example is written the same way as cortex-m's `critical-section-single-core` and as the built-in versions of `embassy-cw32` and `cw32l012-hal`. **One firmware may carry exactly one implementation** (enabling two is a duplicate definition) — this book keeps that guarantee by keeping the three example crates isolated from each other.

Choosing `u8` for the restore state was not written casually: cargo unifies the critical-section features of all crates when it builds the whole workspace, and the `restore-state-*` variants are **mutually exclusive** — two of them in one build fail the compile, so the whole dependency graph admits only one choice. embassy-cw32's current dependency chain is uniformly on `u8` (the implementation remains this crate's PRIMASK version — the `critical-section-impl` feature is on by default — and the book's examples align with that).

## `_defmt_timestamp`: a Number for Every Log Line

defmt lets each log line carry a "timestamp" value. The device side must provide a function whose symbol name is `_defmt_timestamp`; each time a line is logged, defmt calls it and encodes the returned content into the frame; the host shows this value at the start of each decoded line.

In the defmt **0.3** era, the application hand-wrote a raw symbol:

```rust
#[no_mangle]
fn _defmt_timestamp() -> u64 { /* uptime in milliseconds */ }
```

defmt **1.x** replaced it with a macro — and the signature changed (the function receives a `Formatter`, written for you by the macro):

```rust
static SEQ: AtomicU32 = AtomicU32::new(0);
defmt::timestamp!("{=u32}", SEQ.load(Ordering::Relaxed));
```

`timestamp!` may be used only once across the whole crate graph. Omitting it still links: the `defmt.x` linker script shipped with defmt contains `PROVIDE(_defmt_timestamp = __defmt_default_timestamp)` — the fallback is an empty implementation, and when the host sees only the default symbol in the ELF, it simply does not display the timestamp column.

This example has no clock driver, so what it supplies is actually **not time** but a sequence number — "which log line this is"; defmt just carries the number to the host as-is. To see a real millisecond uptime, you need a time source first: the embassy ecosystem's `rtt_hello` provides one through embassy-time (that is the route this book takes once Chapter 7 enters the async world).

One real M0+ limitation worth a mention in passing: the `SEQ` counter cannot be written as `SEQ.fetch_add(1, …)` — the Cortex-M0+ has no atomic read-modify-write instruction, and `core`'s `fetch_add` **does not exist** on the `thumbv6m-none-eabi` target (`load`/`store` do). The example takes the three-step route of load, add one, store; on a single core with interrupts still off, this is safe.

## The Full Example

`examples/pac/examples/ch03_rtt.rs`:

```rust
//! Chapter 3 example: RTT logging — defmt frames travel to the debugger over
//! the SEGGER-RTT upstream channel, decoded natively by `probe-rs run`,
//! occupying no UART pin at all.
//!
//! The delay and the heartbeat LED use the `board` wrapper born in Chapter
//! 2's engineering section (`board::delay_ms` / `board::Led`): log lines and
//! LED toggles stay in sync, so on a first board run "the LED blinks but no
//! log scrolls by" points at the RTT link.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch03_rtt -p cw32-book-pac
//! Status: compiles, not yet run on hardware. The dependency combination
//! (defmt 1 + defmt-rtt 0.4 + panic-probe(print-defmt)) is identical to
//! embassy-cw32's rtt_hello, whose commit record lists the verification
//! level as "release link clean" (link passes).

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m_rt::entry;
use cw32_book_pac::board::{Led, delay_ms};
use cw32l012_pac_um::Peripherals;
use defmt_rtt as _; // global logger: defmt frames into the RTT upstream channel
use panic_probe as _; // panic handler: report the location via defmt, then halt

// defmt 1.x defines the "timestamp provider" with the timestamp! macro: the
// macro expands at top level into a #[export_name = "_defmt_timestamp"]
// function, and since defmt-rtt does not provide one, linking depends on
// it. In the defmt 0.3 days you hand-wrote #[no_mangle] fn
// _defmt_timestamp() -> u64; from defmt 1.x the signature takes a
// Formatter, written by the macro.
// This example has no time driver, so the timestamp is just "which log
// line this is" — defmt only carries the number to the host as-is. Real
// millisecond uptime needs a time source (embassy's rtt_hello provides one
// through embassy-time; the route this book takes from Chapter 7 on).
static SEQ: AtomicU32 = AtomicU32::new(0);
defmt::timestamp!("{=u32}", SEQ.load(Ordering::Relaxed));

// defmt-rtt guards its log buffer with critical-section, and bare-metal
// M0+ has no ready-made critical-section implementation — the firmware must
// provide one itself, or the link fails with
// `undefined symbol: _critical_section_1_0_acquire`. The standard
// single-core recipe: read PRIMASK → `cpsid i` to mask interrupts; on
// release, restore only if interrupts were originally enabled.
// Written the same way as the implementations of cortex-m's
// critical-section-single-core and of cw32l012-hal — one firmware may carry
// exactly one implementation; this book keeps that by isolating the example
// crates from each other.
struct SingleCoreCs;
critical_section::set_impl!(SingleCoreCs);

unsafe impl critical_section::Impl for SingleCoreCs {
    unsafe fn acquire() -> critical_section::RawRestoreState {
        let primask: u32;
        unsafe {
            core::arch::asm!("mrs {0}, PRIMASK", "cpsid i", out(reg) primask);
        }
        (primask & 1) as u8 // RawRestoreState = u8 (pinned in dev-dependencies)
    }

    unsafe fn release(token: critical_section::RawRestoreState) {
        if token == 0 {
            unsafe { core::arch::asm!("cpsie i") };
        }
    }
}

#[entry]
fn main() -> ! {
    // Reference the PAC: bring the vector table in with the rlib (the
    // discipline from the PAC's docs/rt-smoke.md)
    let p = Peripherals::take().unwrap();

    // Heartbeat LED: the gating/analog/direction three steps live in
    // board::Led::new (Chapter 2's engineering section)
    let mut led = Led::new(&p);

    defmt::info!("ch03 RTT hello — 复位默认时钟 HSI 4 MHz");

    loop {
        // M0+ has no atomic read-modify-write instruction, and core's
        // fetch_add does not exist on thumbv6m — load/store in three steps
        // is all there is; single core, interrupts not yet enabled: safe.
        let n = SEQ.load(Ordering::Relaxed) + 1;
        SEQ.store(n, Ordering::Relaxed);
        defmt::info!("heartbeat #{=u32}", n);
        led.toggle(); // LED and log lines stay in sync; two channels cross-checking each other
        delay_ms(500);
    }
}
```

The two `use … as _` lines are a recurring idiom in this book's examples: `defmt_rtt` and `panic_probe` work through linked symbols (the global logger, the panic handler) and need no explicit API call; `as _` puts the intent "bring it in" in the most visible spot while producing no unused warnings. The `#{=u32}` in `info!` is defmt's typed formatting: `=u32` declares that the argument is encoded as a u32, and the host reconstructs it accordingly.

The delay and the heartbeat LED come from Chapter 2's `board` wrapper: `board::delay_ms(500)` is a SysTick busy-wait (millisecond-calibrated, replacing the counting-loop delay of this chapter's earlier draft), and `board::Led::new` performs the three pre-lighting steps. Log lines and LED toggles are synchronized — on a first board run, "the LED blinks but no log scrolls by" tells you the problem is in the RTT link, not the main loop.

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch03_rtt -p cw32-book-pac
```

## Running It: Flash and Collect Logs in One Command

probe-rs's `run` subcommand chains "flash, reset, attach RTT, decode defmt" into a single command (probe and chip parameters in Chapter 1; the full quick reference in Appendix D):

```bash
probe-rs run --probe 0483:3748 --protocol swd --chip CW32L012x8 \
    target/thumbv6m-none-eabi/release/examples/ch03_rtt
```

The firmware logs periodically, and decoded lines scroll through the terminal; each line begins with the value returned by the timestamp provider (here, the log sequence number). The expected shape is roughly:

```text
      1 INFO ch03 RTT hello — 复位默认时钟 HSI 4 MHz
      2 INFO heartbeat #1
      3 INFO heartbeat #2
      … (Ctrl-C to exit)
```

(Exact column widths, colors, and timestamp rendering depend on the probe-rs version; trust the actual output. This batch of bytes has not been checked against a real board — on the first hardware run, compare against the checklist in "On-Hardware Verification" below.)

`probe-rs run` exits automatically when `main` returns — this example's `main` never returns, so press Ctrl-C when you have seen enough and the firmware keeps running on the board. To attach without re-flashing, use `probe-rs attach` (same arguments).

## defmt or rprintln?

There is another route to logging over RTT: `rtt-target`'s `rprintln!` — plain UTF-8 text straight out of the chip, readable in the host terminal with no decoding and no ELF needed.

The trade-offs of the two routes:

| | defmt (this chapter) | rprintln (rtt-target) |
|---|---|---|
| Firmware cost | Small: format strings interned into the ELF; frames are compact binary | Large: every line carries its full text |
| Host side | Needs decoding (built into probe-rs) | Readable as-is |
| Level filtering | Compile time, via `DEFMT_LOG` | None; printing happens regardless |
| Panic reporting | `panic-probe` prints the location, then halts | Wire it up yourself |
| Coexists with rtt-target | No (the RTT control block is unique) | — |

The embedded Rust ecosystem's default choice is defmt (all of embassy uses it), and every later chapter of this book logs through defmt too. `rprintln` fits one-off debugging where "you just want a printf".

## On-Hardware Verification

**This example itself: compiles, not yet run on hardware** (thumbv6m-none-eabi release build passes; `nm` confirms that the `_SEGGER_RTT` control block, `_defmt_timestamp`, and the `.defmt` metadata section have all landed, with the vector table intact).

The dependency combination (defmt 1 + defmt-rtt 0.4 + panic-probe(print-defmt)) is identical to embassy-cw32's `rtt_hello`. One thing to be said plainly: `rtt_hello`'s commit record lists its verification level as **"release link clean"** (the thumbv6m release link passes), and it likewise does not claim a board run — there is no on-hardware RTT log record to cite anywhere in this ecosystem yet. On the first hardware run, check the items below one by one; results are welcome back into this book's issues.

First-run checklist:

1. `probe-rs list` sees the probe; `probe-rs run` does not report `RTT control block not found` (the control block lives in SRAM; if the linker optimized it away, check whether the `_SEGGER_RTT` symbol survived);
2. `INFO`-level lines scroll out with the sequence number increasing; if garbled text or missing lines appear, drop `DEFMT_RTT_BUFFER_SIZE` to 512 and retry (buffer-overflow truncation is RTT's most common symptom — defmt-rtt defaults to non-blocking truncation mode);
3. After Ctrl-C, check whether the firmware is still running (expected: it keeps going, with the PC13 heartbeat LED blinking at 1 Hz — it toggles in sync with the log lines). LED blinking but no log: the problem is in the RTT link; go back to items 1 and 2. LED not blinking either: start from GPIOC gating and ANALOG (Chapter 2's two silent traps).

## Common Pitfalls

- **Link fails with `undefined symbol: _critical_section_1_0_acquire`**: there is no critical-section implementation. `set_impl!` one as in this chapter's example; one firmware carries exactly one implementation.
- **`timestamp!` used twice**: its marker symbol (`__DEFMT_MARKER_TIMESTAMP_…`) may be defined only once across the whole crate graph; a duplicate fails at link time with a repeated symbol — put the timestamp provider either in the final firmware crate or in exactly one dependency.
- **`fetch_add` and other RMW atomic operations on thumbv6m**: the build reports method not found — the M0+ has no atomic read-modify-write instructions, and `core` only provides `load`/`store`. When you need full atomic semantics, wrap ordinary reads and writes in `critical_section::with`.
- **`defmt` and `rtt-target` included at the same time**: RTT control block conflict; pick one.
- **Logs "lose their tail"**: defmt-rtt truncates by default when the buffer fills (without blocking the firmware); for high-frequency logs, raise `DEFMT_RTT_BUFFER_SIZE` or lower the print rate.
- **Wanting the timestamp shown as seconds/milliseconds**: the unit is a display-side convention; the device only supplies a number — whatever hint you write in the format string (`{=u32:ms}` and the like), probe-rs renders accordingly.
