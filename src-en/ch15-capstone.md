# Chapter 15 Capstone: A Serial Command Console

In this chapter you will learn:

- How to wire the four building blocks of the previous 14 chapters (GPIO,
  UART, ADC, PWM) into **one runnable project** — a serial command console;
- The three genuinely new integration problems: the combined clock-gating
  word, multi-binding with `bind_interrupts!`, and constructor order versus
  the APBEN1 restore;
- The task split: a command-parsing task owning all the peripherals and a
  zero-peripheral heartbeat task — "ownership is permission";
- The minimal set of cross-task shared state: why accumulating a counter on
  thumbv6m takes a critical section instead of an atomic;
- The minimal directory layout for turning the example into your own project,
  plus the item-by-item first-flash checklist.

## Design: What a Skeleton Project Looks Like

The previous 14 chapters were layered: raw PAC registers (Part 1), the
blocking HAL (Part 2), embassy async (Part 3), peripherals in depth (Part 4).
Each chapter's example demonstrated exactly one thing. The capstone inverts
that: **the features are ordinary, the integration is the real exam** — put
the four building blocks into one firmware and make them alive at the same
time without stepping on each other.

The project is a serial command console: one USB-to-serial cable, and power-up
gives you an interactive shell. The protocol is five lines:

| Command | Effect | Building block (source) |
|---|---|---|
| `led on` / `led off` | PC13 on / off (polarity depends on the board) | `gpio::Output` (ch7) |
| `adc` | Single PA0 sample, reports raw code + mV | `adc::Adc` `read` + `vdda_mv` (ch11) |
| `pwm <0..10000>` | GTIM1@PB04 duty adjusted while running | `timer::GtimPwm` `set_duty_bp` (ch10) |
| `info` | heartbeat count + PWM duty + last ADC sample | two tasks + critical section |
| anything else | echoes the word + prints the command table (error path) | — |

There are just two tasks, on one executor (ch7):

```text
USB-TTL ──UART1──► console task (sole owner of rx/tx/adc/pwm/led)
                     │   rx.read → line assembly → exec() → reply
                     ▼
                     ├─► Output(PC13)        (led command)
                     ├─► Adc(PA0)            (adc command)
                     └─► GtimPwm(PB04)       (pwm command)

heartbeat task (zero peripherals) ──1s──► TICKS +1 ──critical-section──► info reads
```

The `console` task receives **all** the peripherals by ownership; the
`heartbeat` task touches none of them — this split is the core technique of
the chapter, covered in Assembly Point 3. All four peripheral recipes come
from earlier chapters; the only genuinely **new** code is the integration
layer, covered next.

## Assembly Point 1: Clock Gating — Three Ports, One Word; Three Peripherals, One Word

Chapters 7/8 hit this once: AHBEN is a keyed whole-word write (the upper 16
bits must be KEY = 0x5A5A), and two separate writes clobber each other. This
example needs three ports at once — GPIOA (PA0/PA02/PA03), GPIOB (PB04),
GPIOC (PC13), AHBEN bits 4/5/6 (manual §4.7) — so one write must set them
all:

```rust,ignore
p.sysctrl.ahben.write(SYSCTRL_KEY | (1 << 4) | (1 << 5) | (1 << 6));
```

The APBEN1 problem is more hidden; it lives in the constructor order. The
three peripheral drivers **each** turn on their own clock, with different
styles (embassy-cw32 source):

- `BufferedUart::new_uart1`: **whole-word write** of APBEN1 = `KEY | bit3`
  (`src/uart.rs`; UART1 = bit3);
- `Adc::new_adc1`: **whole-word write** of APBEN1 = `KEY | APBEN1_ADC`
  (`src/adc.rs`; the PAC defines `APBEN1_ADC = 1 << 0`);
- `GtimPwm::new`: **read-modify-write** that adds the GTIM1 bit
  (`src/timer.rs`; GTIM1 = bit6).

Called back to back, the first two whole-word writes clobber each other:
after the ADC constructor, the UART1 clock bit is already gone. No
single-peripheral example ever exposed this — every driver constructor
assumes "I am the only one configuring clocks". A multi-peripheral project
has two options: patch the upstream drivers to read-modify-write (beyond
this chapter; a legitimate contribution to the ecosystem repos), or, as this
example does, **write one combined word after all constructors**:

```rust,ignore
p.sysctrl.apben1.write(SYSCTRL_KEY | (1 << 0) | (1 << 3) | (1 << 6));
```

Note that this line restores the **clock**, not the configuration: each
constructor ran with its own clock on, so the register configuration is
already inside the peripheral; once the clock is restored, the configuration
takes effect unchanged. Do not invert the order — configuring after the gate
is closed means writing into a room nobody can hear.

## Assembly Point 2: Multi-Binding with bind_interrupts!

Two interrupt lines are needed here: UART1 (IRQ27, drains the RX pipe) and
ADC1 (IRQ12, wakes on conversion complete). One `bind_interrupts!` struct can
bind several, separated by semicolons:

```rust,ignore
embassy_cw32::bind_interrupts!(struct Irqs {
    UART1 => uart::Uart1Irq;
    ADC1 => embassy_cw32::adc::Adc1Irq;
});
```

The macro expands two strong-symbol vector functions (overriding the weak
defaults in the PAC's `device.x`) and implements two `Binding<...>`s for
`Irqs` — the constructors' compile-time checks keep working as usual (ch9).

GTIM1 is deliberately **not** bound: only `wait_update()` needs the GTIM1
update interrupt (IRQ16); this example only changes the duty with
`set_duty_bp`, which generates no interrupt. Extra bindings would still
compile (the vector function exists, nobody enables the NVIC line, no side
effects — ch9), but "bound yet unused" is noise: the interrupt-binding list
should be an accurate ledger of who is waiting on interrupts.

## Assembly Point 3: Task Split — Ownership Is Permission

The `console` task's signature is the permission list: five parameters — rx,
tx, adc, pwm, led — all moved into the task by `spawner.spawn(...)` (ch7:
spawn arguments are packed into a `SpawnToken`, statically allocated, a
second spawn returns `SpawnError::Busy`). While executing a command, all
five peripherals are held via `&mut` — owned by one task, no races, and
`exec` needs no locks at all.

The `heartbeat` task is the contrast: zero peripherals. It touches only the
`TICKS` counter and RTT logging, proving that "the executor is alive" needs
no hardware peripheral — the ch8 heartbeat LED cannot be used here anyway
(PC13's ownership sits with console; `led on/off` must control it, so the
heartbeat must not flip it).

`main`'s last two lines spawn and exit: when the `#[main]` function body
ends, the executor's scheduling loop takes over (ch7) and keeps polling both
child tasks. The split rule in one sentence: **anything ownership can solve,
do not solve with a lock**. Give every peripheral to one task, and the state
that genuinely needs synchronization surfaces — here, one `TICKS`.

## Assembly Point 4: The Only Cross-Task Shared State

`TICKS` is a u32 counter written only by heartbeat and read by console via
`info`. The intuitive answer is an atomic, but on thumbv6m that does not fly:

- **Hardware**: Cortex-M0 has no LDREX/STREX — atomic read-modify-write
  instructions simply do not exist on ARMv6-M (they arrived with ARMv7-M);
- **Library**: on this repo's toolchain (verified with rustc 1.97), the
  target's `core::sync::atomic::AtomicU32` offers only `load`/`store` —
  `fetch_add`/`swap` do not exist (E0599).

So accumulation is guarded by a critical section — the same recipe as ch14's
static record area (`Mutex<RefCell<u32>>` + `critical_section::with`). The
critical-section implementation comes from embassy-cw32's default features
(PRIMASK save/disable; the defmt-rtt of ch3's trio already uses the same
implementation — a firmware can have exactly one implementation, and ch7
covered the link-time conflict of mixing two).

The upgrade path beyond that is embassy-sync: multi-field state wants
`Mutex<Cell<T>>` (the async Mutex, no critical section), event notification
wants `Signal`. A single counter does not need any of that — **start the
shared state from the minimal set and change tools when it grows**; the
"Next Steps" section has an exercise that genuinely needs `Signal`.

## The Example

The full `examples/embassy/examples/ch15_console.rs` (build command after the
listing):

```rust
//! Chapter 15 example: a serial command console (capstone — wiring the
//! building blocks of the previous 14 chapters into one project).
//!
//! UART1 (PA02=TXD / PA03=RXD, AF1, 115200-8N1, BufferedUart) receives line
//! commands: `led on|off` (PC13; on/off polarity depends on the board), `adc`
//! (single PA0 sample converted to mV via VDDA), `pwm <0..10000>` (GTIM1@PB04
//! duty adjusted while running), `info` (runtime status); unknown commands
//! get an error reply. The console task owns all the peripherals, the
//! heartbeat task only touches a critical-section counter + RTT logging —
//! two tasks on one executor.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch15_console -p cw32-book-embassy
//! Status: compiles, not yet run on hardware. The four peripheral recipes all
//!       come from earlier chapters: BufferedUart (ch8, same-source
//!       verified), ADC (ch11, register recipe same-source as the blocking
//!       version), PWM (ch10, same-source); the integration layer (combined
//!       APBEN1 word, line protocol, task split) is new in this chapter.
//!       First-flash checklist in the Chapter 15 text.

#![no_std]
#![no_main]

use core::{cell::RefCell, fmt::Write as _};

use critical_section::Mutex;
use embassy_executor::{main, task};
use embassy_time::Timer;

use cw32_book_embassy as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use defmt_rtt as _; // RTT log channel (the defmt-rtt of the ch3 trio)
use embassy_cw32::{
    Config, Peripherals, SYSCTRL_KEY,
    adc::{Adc, AdcChannel, AdcClk, AdcError, SampleCycles},
    gpio::{Level, Output, Pin},
    timer::{GtimPwm, PwmChannel, PwmConfig},
    uart::{self, BufferedUart, BufferedUartRx, BufferedUartTx},
};

// Bind both interrupt lines at once: UART1 (IRQ27, drains the RX pipe) +
// ADC1 (IRQ12, wakes on conversion complete). GTIM1 stays unbound — pure
// set_duty usage never touches wait_update, so no interrupt is generated.
embassy_cw32::bind_interrupts!(struct Irqs {
    UART1 => uart::Uart1Irq;
    ADC1 => embassy_cw32::adc::Adc1Irq;
});

/// Heartbeat tick counter: written only by the heartbeat task, read by the
/// console task via the info command — the only cross-task shared state in
/// this example. thumbv6m has no atomic read-modify-write (Cortex-M0 lacks
/// LDREX/STREX; core::sync::atomic offers no fetch_add/swap methods on this
/// target), so accumulation is guarded by a critical section (the same
/// recipe as ch14's static record area).
static TICKS: Mutex<RefCell<u32>> = Mutex::new(RefCell::new(0));

/// Line buffer limit (arguments included). Overlong lines are reported and
/// dropped, so one command cannot flood the input.
const MAX_LINE: usize = 31;

/// Stack formatting buffer: a core::fmt::Write adapter (for building numbers
/// without alloc).
struct Fmt<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl core::fmt::Write for Fmt<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let rest = &mut self.buf[self.len..];
        let n = s.len().min(rest.len());
        rest[..n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// Send a byte string and wait until it is on the wire (flush waits for TC:
/// the last byte has fully shifted out)
async fn reply(tx: &mut BufferedUartTx, msg: &[u8]) {
    tx.write(msg).await;
    tx.flush().await;
}

/// Format one reply and send it: stack buffer + format_args! for the numbers
/// (truncated if the buffer is too short)
async fn reply_fmt(tx: &mut BufferedUartTx, out: &mut [u8], args: core::fmt::Arguments<'_>) {
    let mut f = Fmt { buf: out, len: 0 };
    let _ = f.write_fmt(args);
    reply(tx, &f.buf[..f.len]).await;
}

/// AdcError → short label (error paths must stay readable, not swallowed)
fn adc_err(e: AdcError) -> &'static str {
    match e {
        AdcError::Timeout => "timeout",
        AdcError::BadCalibration => "bad calibration",
        AdcError::UnsupportedChannel => "unsupported channel",
        AdcError::SensorDead => "sensor dead",
    }
}

/// Parse and execute one line. Every peripheral is accessed exclusively via
/// &mut — owned by a single task, no races.
async fn exec(
    tx: &mut BufferedUartTx,
    led: &mut Output<embassy_cw32::Gpio>,
    adc: &mut Adc,
    pwm: &mut GtimPwm,
    last: &mut [u32; 2], // [0]=VDDA mV, [1]=PA0 raw code (shown by the info command)
    line: &str,
) {
    // Split at the first space: cmd + argument (whitespace tolerated around it)
    let (cmd, arg) = match line.split_once(' ') {
        Some((c, a)) => (c, a.trim()),
        None => (line, ""),
    };
    match cmd {
        "led" => match arg {
            "on" => {
                led.set_low(); // polarity depends on the board (ch4): record it at first flash
                reply(tx, b"led: on\r\n").await;
            }
            "off" => {
                led.set_high();
                reply(tx, b"led: off\r\n").await;
            }
            _ => reply(tx, b"usage: led on|off\r\n").await,
        },
        "adc" => {
            // VDDA first: internal 1.2V reference + factory calibration (ch11
            // recipe), used as the conversion denominator
            let mv = match adc.vdda_mv().await {
                Ok(mv) => mv,
                Err(e) => {
                    let mut out = [0u8; 48];
                    reply_fmt(tx, &mut out, format_args!("adc: VDDA error ({})\r\n", adc_err(e)))
                        .await;
                    return;
                }
            };
            // Single conversion on PA0 (external channel IN0; readings float
            // when the pin is unconnected, follow the input with a divider)
            match adc.read(AdcChannel::In(0), SampleCycles::S24).await {
                Ok(raw) => {
                    last[0] = mv;
                    last[1] = raw as u32;
                    // 12-bit full-scale conversion: raw/4095 × VDDA
                    let pin_mv = (raw as u32 * mv) / 4095;
                    let mut out = [0u8; 72];
                    reply_fmt(
                        tx,
                        &mut out,
                        format_args!("adc: PA0 raw={} ~{} mV (VDDA {} mV)\r\n", raw, pin_mv, mv),
                    )
                    .await;
                }
                Err(e) => {
                    let mut out = [0u8; 48];
                    reply_fmt(tx, &mut out, format_args!("adc: PA0 error ({})\r\n", adc_err(e)))
                        .await;
                }
            }
        }
        "pwm" => match arg.parse::<u16>() {
            Ok(d) => match pwm.set_duty_bp(d) {
                Ok(()) => {
                    let rb = pwm.duty_bp(); // readback computed from CCR/ARR (ARPE preloading)
                    let mut out = [0u8; 64];
                    reply_fmt(
                        tx,
                        &mut out,
                        format_args!("pwm: duty={}/10000 (readback {})\r\n", d, rb),
                    )
                    .await;
                }
                Err(_) => reply(tx, b"pwm: duty must be 0..=10000\r\n").await,
            },
            Err(_) => reply(tx, b"usage: pwm <0..10000>\r\n").await,
        },
        "info" => {
            let ticks = critical_section::with(|cs| *TICKS.borrow_ref(cs));
            let mut out = [0u8; 96];
            reply_fmt(
                tx,
                &mut out,
                format_args!(
                    "info: ticks={} | pwm={}/10000 | last adc: raw={} vdda={} mV\r\n",
                    ticks,
                    pwm.duty_bp(),
                    last[1],
                    last[0]
                ),
            )
            .await;
        }
        _ => {
            // Unknown command: echo the word back, then the command table —
            // error paths deserve friendliness too
            reply(tx, b"unknown command: ").await;
            reply(tx, cmd.as_bytes()).await;
            reply(tx, b"\r\ntry: led on|off | adc | pwm <0..10000> | info\r\n").await;
        }
    }
}

/// Console task: read a byte stream → assemble lines → dispatch. rx/tx/adc/
/// pwm/led are all moved into this task by spawn — ownership is permission.
#[task]
async fn console(
    mut rx: BufferedUartRx,
    mut tx: BufferedUartTx,
    mut adc: Adc,
    mut pwm: GtimPwm,
    mut led: Output<embassy_cw32::Gpio>,
) {
    reply(&mut tx, b"\r\n== CW32L012 serial console (Rust on CW32 ch15) ==\r\n").await;
    reply(&mut tx, b"try: led on|off | adc | pwm <0..10000> | info\r\n").await;

    let mut chunk = [0u8; 32];
    let mut line = [0u8; MAX_LINE];
    let mut len = 0usize;
    let mut overflow = false;
    let mut last = [0u32; 2];

    loop {
        let n = rx.read(&mut chunk).await; // parks until at least 1 byte is in the pipe
        tx.write(&chunk[..n]).await; // echo the whole chunk (for terminals without local echo)
        for &b in &chunk[..n] {
            if b == b'\r' || b == b'\n' {
                // End of line (most terminals send CR, some LF; with CRLF the
                // LF lands on len==0 and is naturally ignored)
                if len > 0 || overflow {
                    tx.write(b"\r\n").await;
                    if overflow {
                        reply(&mut tx, b"error: line too long\r\n").await;
                    } else {
                        let s = core::str::from_utf8(&line[..len]).unwrap_or("");
                        exec(&mut tx, &mut led, &mut adc, &mut pwm, &mut last, s).await;
                    }
                    len = 0;
                    overflow = false;
                }
            } else if len < MAX_LINE {
                line[len] = b;
                len += 1;
            } else {
                overflow = true; // overlong: swallow silently, report once at end of line
            }
        }
    }
}

/// Heartbeat task: a tick every 1s. Touches no peripherals — the TICKS
/// counter (read by the console's info command) + defmt RTT logging (visible
/// when probe-rs attaches).
#[task]
async fn heartbeat() {
    loop {
        Timer::after_millis(1000).await;
        critical_section::with(|cs| *TICKS.borrow_ref_mut(cs) += 1);
        defmt::info!("heartbeat tick");
    }
}

#[main]
async fn main(spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // reset-default SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // Three ports in one write: AHBEN is a keyed whole-word write (upper 16
    // bits must be 0x5A5A); separate writes clobber each other (same
    // treatment as ch10/11). GPIOA(bit4, PA0/PA02/PA03), GPIOB(bit5, PB04),
    // GPIOC(bit6, PC13)
    p.sysctrl.ahben.write(SYSCTRL_KEY | (1 << 4) | (1 << 5) | (1 << 6));

    // LED: High after reset (on/off polarity depends on the board, ch4)
    let led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    // UART1 (ch8 recipe): the constructor whole-word writes APBEN1 = KEY|UART1
    let u = BufferedUart::new_uart1(
        p.uart1,
        Pin::new(p.gpioa, 2),
        Pin::new(p.gpioa, 3),
        1, // AF1
        Irqs,
        Default::default(), // uart::Config: 115200
    );
    let (mut tx, rx) = u.split();

    // ADC1 (ch11 recipe): its constructor whole-word writes APBEN1 = KEY|ADC —
    // the UART1 bit from above is gone (restored at the end, see below).
    // PA0 → analog mode (IN0)
    let adc = Adc::new_adc1(p.adc1, AdcClk::Div2, Irqs);
    let _pa0 = Pin::new(p.gpioa, 0).into_analog();

    // GTIM1 PWM (ch10 recipe): its constructor read-modify-writes APBEN1,
    // adding the GTIM1 bit
    let pwm = match GtimPwm::new(
        p.gtim1,
        Pin::new(p.gpiob, 4),
        6, // GTIM1_CH1@PB04 = AF6 (manual Table 9-2)
        PwmConfig {
            channel: PwmChannel::Ch1,
            freq_hz: 1_000,
            duty_bp: 0, // 0% at power-up: wait for the `pwm` command
        },
    ) {
        Ok(t) => t,
        Err(_) => {
            // Cannot trigger with this example's constants (1kHz@4MHz is in
            // range); a real failure reports and halts
            reply(&mut tx, b"fatal: pwm init failed\r\n").await;
            loop {
                core::hint::spin_loop();
            }
        }
    };

    // APBEN1 wrap-up: each constructor wrote its own (two whole-word writes
    // clobbering each other + one read-modify-write); one combined word at
    // the end lines up all three enable bits (ADC=bit0, UART1=bit3,
    // GTIM1=bit6, manual §4). What is restored is the clock — the peripheral
    // register configuration was already written during construction.
    p.sysctrl.apben1.write(SYSCTRL_KEY | (1 << 0) | (1 << 3) | (1 << 6));

    // Two tasks: console owns all the peripherals (ownership moved in via
    // spawn), heartbeat only touches TICKS + RTT. main exits here — the
    // executor run() loop expanded by #[main] never returns; both child
    // tasks keep being polled.
    spawner.spawn(console(rx, tx, adc, pwm, led).unwrap());
    spawner.spawn(heartbeat().unwrap());
}
```

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch15_console -p cw32-book-embassy
```

A few details worth a pause:

**Line assembly**. `rx.read(&mut chunk)` takes a chunk from the pipe (parks
until at least 1 byte, ch8); the chunk is echoed whole, then assembled byte
by byte: `\r`/`\n` end a line (most terminals send CR, some LF; with CRLF the
LF lands on an empty line and is naturally ignored), ordinary bytes append to
`line`. Overlong lines (>31 bytes) are swallowed silently and reported once
at end of line as `error: line too long` — a limit without an error is the
more dangerous half: truncation would turn `led onxxxx` into a different
command.

**Building numbers without alloc**. Formatted replies go through
`reply_fmt`: a stack byte array + `Fmt` (implementing `core::fmt::Write`) +
`format_args!`, truncated if the buffer is short. Without alloc there is no
`String`, so the `write!`/`format_args!` machinery needs a `Write` that
lands in a byte array — a dozen lines, written once, used everywhere.

**PWM adjusted while running**. `pwm 5000` → `set_duty_bp(5000)` writes CCR
(ARPE preloading, effective the next update cycle, ch10); `duty_bp()`
computes the readback from CCR/ARR — the readback in the reply is the
evidence that the register really changed.

**Every error path talks**. Unknown commands echo the word + the command
table; invalid arguments get a usage line; every ADC error variant is
translated by `adc_err` (four variants, including `SensorDead`, ch11's
temperature-sensor individual difference); a PWM constructor failure (this
example's constants cannot trigger it) reports fatal and halts. Errors are
not swallowed, not guessed away, not hidden behind a reset.

## From Example to Project: A Directory Layout

Turning the example into your own project takes three minimal steps: create
the project directory, copy the dependency list, move `main.rs`.

```text
my-console/
├── .cargo/config.toml   # target = thumbv6m-none-eabi; probe-rs runner (Appendix D)
├── Cargo.toml           # copy the [dependencies] section of examples/embassy/Cargo.toml
└── src/
    ├── main.rs          # ch15_console.rs verbatim is the starting point
    ├── protocol.rs      # exec() and the command table — the first thing to split out
    └── codec.rs         # Fmt / reply / reply_fmt — line assembly and replies
```

Two rules: **do not split yet** — splitting a single file under 300 lines is
a net loss; split `protocol.rs` when the commands outgrow one screen.
**Do not add dependencies** — the example's dev-dependencies
(embassy-executor/embassy-time/the defmt trio/critical-section) are the
entire inventory of the embassy route; check Appendix B for something
ready-made before adding anything.

Flash and observe:

```bash
probe-rs run --chip CW32L012x8 target/thumbv6m-none-eabi/release/examples/ch15_console
```

`probe-rs run` flashes + resets + attaches RTT automatically (Appendix D);
the heartbeat task's `defmt::info!` ticks print straight to the terminal.

## On-Hardware Verification

**Status: compiles, not yet run on hardware.** Each of the four peripheral
recipes has its own verification record in earlier chapters (BufferedUart
same-source verified; ADC/PWM register recipes same-source as the blocking
versions), but **this combined shape has never been flashed as a whole**: the
integration layer (the combined APBEN1 word, the dual interrupt binding, the
two tasks, the line protocol) is new code from this chapter with no on-hardware
record. First-flash checklist, in order:

- **Hardware prep**: USB-to-serial adapter crossed (adapter RXD → board
  PA02, adapter TXD → board PA03, common GND, same as ch5); a potentiometer
  divider on PA0 (optional; floating readings are nondeterministic and
  normal); scope or logic analyzer on PB04 (optional); probe-rs probe on
  SWD for RTT.
- **Checklist** (power up, then run in order):
  1. After reset the terminal shows the banner and the `try:` line —
     BufferedUart TX path + flush (TC waits for "the last byte fully
     shifted out") connected;
  2. `led on` / `led off` → PC13 turns on/off with correct replies (and
     record this board's polarity: does on mean `set_low` or `set_high`;
     ch4's rule is "depends on the board");
  3. `adc` → VDDA lands in 3200~3300 mV (hal repo TESTING.md measured
     3282 mV, ch11); with a divider on PA0, raw and mV follow the input;
  4. `pwm 5000` → PB04 shows 1 kHz at about 50% duty; `pwm 0` / `pwm 10000`
     → constant low / constant high; `pwm 10001` → usage hint;
  5. `info` → `ticks` increments once per second, in step with the
     `heartbeat tick` lines in the RTT terminal (proof both tasks run);
  6. `hello` → `unknown command: hello` + the command table;
  7. paste a line longer than 31 characters → `error: line too long`;
  8. no banner at all: check TX/RX crossing and common ground first, then
     115200-8N1 (same as ch5), then Appendix C.

## Common Pitfalls

- **Forgetting the APBEN1 wrap-up word**: not a single banner arrives —
  UART1's clock bit was clobbered by the ADC constructor's whole-word write;
  the TX pipe has data but no clock to shift it out. The most
  counter-intuitive spot in this example: one line to fix, an afternoon to
  find.
- **Reaching for embassy-sync by default**: an async Mutex around a u32
  counter is a sledgehammer; critical-section + `RefCell` is shorter and the
  implementation already exists. Conversely, with multi-field state or
  "waiting for an event across tasks", do not hand-roll flag polling — go
  straight to embassy-sync.
- **No line-buffer limit**: writing past the `line` array (memory corruption
  or panic); or the opposite — a limit without an error, where an overlong
  command is silently truncated into a different command.
- **After a frequency change, everything must be recomputed**: this example
  runs entirely at the reset-default 4 MHz; after raising the clock, both
  the UART baud rate and the 1 kHz PWM ARR/PSC are derived from the current
  PCLK by the drivers and must be reconfigured — raising clocks is not part
  of this book's route; re-read the reset-default discussion in ch1/2 first.
- **Putting the ADC command in a tight loop**: `vdda_mv()` internally waits
  for the BGR settling time; one `adc` command takes milliseconds. Invisible
  in a console; an accident inside a 1 ms poller.

## Next Steps

With the skeleton in hand, three exercises in rising difficulty:

1. **"Breathing LED" rework**: move `pwm` to a third `breath` task; when
   console receives `breathe on/off` it notifies via an embassy-sync
   `Signal` — the first time the ownership model **genuinely requires**
   embassy-sync (the gap left by Assembly Points 3/4), and a feel for how
   adding one shared peripheral makes the synchronization structure grow.
2. **ADC sequence + DMA**: upgrade the `adc` command to a two-channel
   `read_sequence` (ch11), then offload with `read_sequence_dma` (ch12).
3. **Scheduled commands via RTC**: execute a preset command on an alarm
   (ch13) — the console grows from "typed by humans" to "used by machines".

The two porting-and-ecosystem questions (what changes when you switch
families, which crates save work) are answered in Appendices A and B. This
book ends here — go hook your console up to some sensors and let the CW32
take a shift for you.
