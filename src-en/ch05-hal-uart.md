# Chapter 5: UART Transmit and Receive

An LED can only blink; a serial port can talk. This chapter gets UART1
running: after reset it prints one welcome line, then echoes back every byte
you send — the foundation for every debugging technique that follows.

**In this chapter you will learn:**

- How to pick the UART1 pins: PA02(TX)/PA03(RX) on AF1 (measured on the
  user board);
- The divider formula behind the baud rate registers `BRRI`/`BRRF`, and
  where the error for 115200 at 4 MHz comes from;
- The `Uart` blocking send/receive API and receive-error self-healing
  (ORE/NE/PE/FE);
- What the failures "baud rate off by a step" and "pin AF not switched" each
  look like.

## 5.1 Pins and Alternate Functions: Read Table 9-2 Before Wiring

The CW32L012 has three UARTs — UART1/2/3 — and each one offers several pin
options (manual Table 9-2; a UART may have several pin groups, but a given
physical pin can belong to only one UART at any moment):

| UART | Option A | Option B |
|------|--------|--------|
| UART1 | **PA02=TXD / PA03=RXD (AF1)** | PA09=TXD / PA10=RXD (AF1) |
| UART2 | PA02=TXD / PA03=RXD (AF2) | PA07=TXD / PA06=RXD (AF1) |
| UART3 | PA04=TXD / PA05=RXD (AF1) | PA01=TXD / PA00=RXD (AF2) |

This book and the `cw32l012-hal` examples uniformly use **UART1 option A:
PA02(TX)/PA03(RX), AF1** — the measured configuration of the user board.
Note that the physical pair PA02/PA03 hangs under both UART1 (AF1) and
UART2 (AF2): switch to the wrong AF number and the registers look perfectly
normal while it is UART2 talking on the wire — the scope shows a waveform,
and the serial terminal shows nothing but garbage.

Pin configuration reuses the typestate `Pin` from Chapter 4: TX as an
alternate-function output, RX as an alternate-function input:

```rust,ignore
let tx = Pin::new(p.gpioa, PortId::A, 2, &mut rcc).into_af_output(Af::Af1);
let rx = Pin::new(p.gpioa, PortId::A, 3, &mut rcc).into_af_input(Af::Af1);
```

`into_af_input` enables the internal pull-up automatically while switching
the AF — the standard arrangement recommended by manual Table 21-1 (digital
function, pull-up input, alternate function), which keeps the pin from
floating and sampling noise frames when the far end is disconnected or the
line is idle.

## 5.2 Baud Rate: BRRI × 16 + BRRF

The UART's baud rate clock UCLK defaults to PCLK (`CR1.SOURCE=00`), with
16× oversampling (`CR1.OVER=00`); the divider formula (manual §21):

```text
Baud = UCLK / (BRRI × 16 + BRRF)
```

`BRRI` is the integer part (16 bits) and `BRRF` is the fractional part
(4 bits), written into two separate registers. The HAL's `calc_brr` computes
`div = round(UCLK/Baud)` with rounding, then splits it into
`BRRI = div/16` and `BRRF = div%16`. A few values backed by unit tests:

| PCLK | Target baud | BRRI | BRRF | Actual baud | Error |
|------|----------|------|------|----------|------|
| 4 MHz | 115200 | 2 | 3 | 114286 | −0.79% |
| 96 MHz | 115200 | 52 | 1 | 115246 | +0.04% |
| 4 MHz | 250000 | 1 | 0 | 250000 | 0 (divides exactly) |
| 4 MHz | 300000 | — | — | `calc_brr` returns Err → panic | |

Look at the first row: 4 MHz ÷ 115200 = 34.7, rounded to 35, giving an
actual baud rate of 114286 — an error of −0.79%. Asynchronous serial
tolerates roughly ±2% on each side, so −0.79% is safe. But if you believe
PCLK is 4 MHz while the firmware actually runs at some other frequency (say
you forgot you had changed a divider), the error immediately leaves the
tolerance window — **a wrong baud rate** usually shows up as one of two
things: every character garbled (divider far off), or occasional frame
errors (divider slightly off, the sampling point grazing the edges). For an
unrealizable baud rate (`BRRI` would be 0 when `PCLK/16 < baud`),
`Uart::new` panics outright — better to blow up at startup than to quietly
run wrong.

## 5.3 Constructing Uart and the Send/Receive API

`Uart::new` identifies UART1/2/3 automatically from the base address of the
register block you pass it, and automatically enables the corresponding APB1
clock gate (UART1 = `APBEN1[3]`; the write-protection key details are as in
Chapter 4). It then stops TX/RX → writes `BRRI`/`BRRF` → clears flags →
enables TX/RX, fixed at 8-N-1:

```rust,ignore
let mut uart = Uart::new(p.uart1, tx, rx, &mut rcc, clocks.pclk_hz, 115_200);
```

Note the fourth and fifth arguments: `clocks.pclk_hz` is the PCLK from the
Chapter 4 `clocks()` snapshot, and the formula uses it as UCLK — **pass the
wrong frequency and the baud rate is necessarily wrong**.

The send/receive API at a glance (all blocking or polling; no interrupts,
no queues):

| Method | Behavior |
|------|------|
| `write_byte(b)` | Waits for `ISR.TXBE` (transmit empty), then writes `TDR` |
| `write_bytes(&[u8])` / `write_str(&str)` | Blocking byte-by-byte transmit |
| `flush()` | Waits for `ISR.TC` (everything, including the stop bit, shifted out) |
| `read_byte()` | Blocks waiting for `ISR.RC`, reads `RDR` to fetch the byte |
| `try_read_byte()` | Returns `None` when no data — friendly to main-loop polling |
| `take_error()` | Decodes and clears receive errors, in priority order overflow > frame > parity > noise |

The receive path has a self-healing design worth noticing: when
`read_byte`/`try_read_byte` finds any error flag latched in `ISR` — ORE
(overflow)/NE/PE/FE — it immediately clears the flag and discards the
residual data in `RDR`. The consequence of not clearing ORE: once an
overflow flag latches and is never cleared, in some implementations the
receive path stops setting `RC` for good, presenting as "the port receives
at first, then plays dead forever after one burst". The HAL defuses this
overflow-induced lockup at the driver level.

## 5.4 The Example: ch05_uart_echo.rs

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch05_uart_echo -p cw32-book-hal
```

The full example (`examples/hal/examples/ch05_uart_echo.rs`):

```rust
//! Chapter 5 example: UART1 echo (PA02=TXD, PA03=RXD, AF1, 115200-8-N-1).
//!
//! Line-for-line identical in source to cw32l012-hal examples/uart_echo.rs
//! (only changed to #[unsafe(no_mangle)] per edition 2024, with adjusted
//! comments). Wiring: MCU PA02 -> USB serial adapter RXD, MCU PA03 <- USB
//! serial adapter TXD, common ground. After reset it sends a welcome
//! message, then echoes every byte received verbatim.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch05_uart_echo -p cw32-book-hal
//! Status: compiles, not yet run on hardware (the ecosystem repo's uart_echo
//!       itself is not in the TESTING.md verified-on-hardware table;
//!       the TX write path does have measured ecosystem corroboration:
//!       waveform seen on PA02, 13 bytes written).

#![no_std]
#![no_main]

use cw32_book_hal as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use cw32l012_hal::{
    gpio::{Af, Pin, PortId},
    rcc::Rcc,
    uart::Uart,
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

    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks(); // PCLK = 4MHz

    // UART1 pins: PA02 = UART1_TXD, PA03 = UART1_RXD (AF1)
    let tx = Pin::new(p.gpioa, PortId::A, 2, &mut rcc).into_af_output(Af::Af1);
    let rx = Pin::new(p.gpioa, PortId::A, 3, &mut rcc).into_af_input(Af::Af1);

    let mut uart = Uart::new(p.uart1, tx, rx, &mut rcc, clocks.pclk_hz, 115_200);

    uart.write_str("CW32L012 Rust HAL ready\r\n");

    loop {
        if let Some(b) = uart.try_read_byte() {
            uart.write_byte(b);
        }
    }
}
```

Wiring (USB-TTL serial adapter, **TX/RX crossed**):

```text
MCU PA02 (TXD) ──────> adapter RXD
MCU PA03 (RXD) <────── adapter TXD
GND           ───────  GND (common ground required)
```

After flashing, open the corresponding COM port in a serial terminal at
115200-8-N-1: you first receive one line, `CW32L012 Rust HAL ready`, and
then whatever you type comes back. The echo loop polls with
`try_read_byte` rather than blocking in `read_byte`; the behavior is
equivalent — this example's main loop has nothing else to do, and you are
welcome to swap the `if let` for `read_byte` to feel the difference between
the two styles.

## 5.5 Verification on Hardware

**Status: compiles, not yet run on hardware.** Stating the evidence
honestly:

- The "verified on hardware" table of cw32l012-hal's `TESTING.md` contains
  **no** uart_echo entry; the README's "unverified" list explicitly names
  "the UART transmit/receive data path", and the roadmap item "on-hardware
  UART TX/RX verification (uart_echo pair-up test)" is likewise unchecked.
- The corroborating evidence that does exist: the `TESTING.md` ecosystem
  entry of 2026-10-24 (embedded-io write path, measured) confirms
  `IO_WRITTEN=13` — the blocking write-byte path underneath `Uart` really
  did emit 13 bytes on PA02, with the waveform observed; but **the full echo
  path (including RX) has not yet been verified against a USB-TTL adapter**.

On the first hardware run, check in this order:

1. Receive only, do not type yet: on power-up the welcome line should arrive
   first — this verifies the TX path, the baud rate, and the COM port
   settings;
2. If the welcome line does not arrive: first check that TX/RX are crossed
   and that the grounds are common, then check that the terminal's baud rate
   is 115200;
3. Welcome line present but the echo is garbled: an RX-path problem — focus
   on whether PA03's AF1 is configured and whether the adapter's TXD really
   reaches PA03;
4. No waveform at all: look at PA02 with an oscilloscope/logic analyzer; no
   waveform means go back and check the AF switching and the APBEN1 gate.

## 5.6 Common Pitfalls

- **Pin AF not switched**: `Uart::new` only handles the UART peripheral
  itself; the pins must first go through
  `into_af_output/into_af_input(Af::Af1)`. With the AF still at 0 (GPIO),
  the peripheral looks perfectly "normal" but no signal leaves the pin.
- **Wrong AF number**: PA02/PA03 carry two functions, UART1 (AF1) and
  UART2 (AF2); the peripheral and the AF number must go together — a mix-up
  means garbage.
- **Clock gating**: the UART clock sits in APBEN1 (key-protected like
  AHBEN). `Uart::new` handles it automatically; when hand-writing registers
  around the HAL, do not forget the key value 0x5A5A.
- **Wrong pclk_hz**: the fifth argument of `Uart::new` is "the PCLK you
  think you have"; when it disagrees with the actual frequency the baud rate
  drifts proportionally — common after changing the clock and forgetting to
  sync.
- **TX-only, or "playing dead"**: hand-rolled code that clears error flags
  but misses ORE jams the receive path on the overflow flag; the HAL's
  `read_byte`/`try_read_byte` clear it automatically.
- **Not crossed / no common ground**: TX to TX never receives anything;
  without the GND reference level you get occasional frame errors or
  intermittent bursts of garbage.
