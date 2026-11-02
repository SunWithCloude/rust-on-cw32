# Chapter 8 Async GPIO and UART

**In this chapter you will learn:**

- The `wait_for_*` waiting family of `Input`, and the per-bit wakeup mechanism
  behind "one interrupt per port";
- The producer/consumer structure of `BufferedUart`: the IRQ drains RDR into a
  pipe, the task takes data out of the pipe;
- The trade-offs between the byte-by-byte `Uart` and `BufferedUart`
  (CPU load / burst tolerance / throughput / RAM);
- Two-task concurrency: an echo task and a heartbeat task running on the same
  executor without blocking each other;
- Building the two examples and their on-hardware verification status.

The two examples correspond to two files: `ch08_gpio_wait.rs` (waiting for a
button edge + LED toggle) and `ch08_uart_buffered.rs` (buffered UART echo +
heartbeat).

## 8.1 Async Input: Waiting for an Edge

In Part 2, scanning a button meant polling `get_level()` in a loop. The
`Input` type of embassy-cw32 offers a different posture — suspend the task,
and let the interrupt wake it when the edge arrives:

```rust,ignore
let mut btn = Input::new(Pin::new(p.gpioa, 0), Pull::Up);
btn.wait_for_falling_edge().await; // up to this line the CPU has been elsewhere (or asleep)
```

The waiting family has five methods, all from `embassy-cw32/src/gpio.rs`:

- `wait_for_rising_edge()` / `wait_for_falling_edge()` /
  `wait_for_any_edge()`: wait for a rising / falling / any edge;
- `wait_for_high()` / `wait_for_low()`: wait for a level. The implementation
  is "edge trigger + level recheck" — read IDR first; if the level does not
  match, wait for any edge and recheck, until the target level shows up.

`Input::new(pin, pull)` configures the pin as a digital input (DIR=1,
ANALOG=0) and sets the pull-up/pull-down. One hardware fact to remember about
pull-downs: **on the L012 only PF3 has a real pull-down resistor** (manual
§9.6.3); on every other pin `Pull::Down` behaves like `Pull::None` — hence the
standard "button to GND" wiring goes with `Pull::Up`.

**One interrupt per port.** CW32 GPIO interrupts are shared per port:
GPIOA/B/C/F each own one IRQ (IRQ5/6/7/8, manual interrupt table). The common
implementation inside the five waiting APIs (`wait_edge`) does the following:

1. Enables the NVIC interrupt line for the port's slot
   (`nvic_enable::<GPIOA>()` and friends, writing ISER);
2. Clears leftover interrupt flags and the software-trigger bitmap, then sets
   the rising/falling edge enables for that pin
   (`GPIOx_RISEIE`/`FALLIE`);
3. Registers a per-pin waker and suspends the task. When the port IRQ arrives:
   merge the pending bits from the ISR into the software-trigger bitmap, write
   ICR to clear the hardware flags, and wake the waiting wakers bit by bit;
4. Once the edge has arrived, automatically turns off that pin's edge enable
   — no interrupts left running in the background.

Unlike UART, the NVIC enable and the dispatch for GPIO are already in place
inside the driver, **but the vector function must be bound by you**: without a
`bind_interrupts!` for GPIOA, the interrupt lands in the weak default
`DefaultHandler` reserved by the PAC's `device.x` (an endless loop) — the task
then suspends forever, with no error and no log. This is exactly what
Chapter 9 is entirely about.

## 8.2 Example 1: ch08_gpio_wait.rs

The button (or a jumper wire) sits between PA0 and GND; the on-board LED is on
PC13. A press = a falling edge on PA0 = one LED toggle.

```rust
//! Chapter 8 example 1: asynchronously wait for a button edge, toggle the LED on PC13.
//!
//! The button (or a jumper wire) sits between PA0 and GND: PA0 is a pull-up
//! input, so a press = a falling edge. `Input::wait_for_falling_edge()` suspends
//! the task; only when the GPIOA port interrupt (IRQ5) arrives does it wake —
//! while waiting, the CPU sleeps in the executor.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch08_gpio_wait -p cw32-book-embassy
//! Status: compiles, not yet run on hardware (first-run checklist in the Chapter 8 text).

#![no_std]
#![no_main]

use embassy_executor::main;

use cw32_book_embassy as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use embassy_cw32::{
    Config, Peripherals, SYSCTRL_KEY,
    gpio::{Input, Level, Output, Pin, Pull},
};

// Only the port being waited on needs an interrupt binding: GPIOA carries
// the Input wait. The LED is an Output and generates no interrupts;
// GPIOC needs no binding.
embassy_cw32::bind_interrupts!(struct Irqs {
    GPIOA => embassy_cw32::gpio::GpioAIrq;
});

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // reset-default SysClk = 4MHz

    let p = Peripherals::take().unwrap();
    // AHBEN is a key-protected whole-word write (upper 16 bits must be 0x5A5A,
    // otherwise the write is discarded): GPIOA(bit4) and GPIOC(bit6) must go
    // into the same write — written one bit at a time, the second write
    // clears the bit the first one just set
    p.sysctrl.ahben.write(SYSCTRL_KEY | (1 << 4) | (1 << 6));

    let mut btn = Input::new(Pin::new(p.gpioa, 0), Pull::Up);
    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    loop {
        btn.wait_for_falling_edge().await; // suspended, waiting for the GPIOA port interrupt
        led.toggle();
    }
}
```

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch08_gpio_wait -p cw32-book-embassy
```

Three details worth pausing on:

**Two AHBEN bits in one write.** This example needs GPIOA (button) and GPIOC
(LED) at the same time, and Chapter 7 established that AHBEN is a
key-protected whole-word write — `sysctrl_enable_gpioa()` and
`sysctrl_enable_gpioc()` each overwrite the full word, so calling the second
clears the first. Hence the direct single-register write here:
`SYSCTRL_KEY | (1 << 4) | (1 << 6)` (GPIOA=bit4, GPIOC=bit6; manual §4.7).

**Bind only the port you wait on.** The `bind_interrupts!` here lists only
GPIOA: the LED is an `Output` and generates no interrupts; binding GPIOC
would buy nothing. The macro expansion generates a strong-symbol vector
function `GPIOA()` (the entry point of IRQ5), dissected line by line in
Chapter 9.

**Debouncing.** The press/release of a mechanical button comes with a few
milliseconds of bounce; a single press can trigger several falling edges and
toggle the LED several times in a row. The classic debounce in async style is
"after the edge, `Timer::after_millis(50).await` and re-verify the level" —
adapting this example that way is left as an exercise.

## 8.3 Example 2: ch08_uart_buffered.rs

`BufferedUart` is a UART driver with background buffering (the design draws on
fan2nd/cw32-hal2; an embassy-sync `Pipe` serves as the ring buffer, 64 bytes
each way). The data flow is two one-way pipes:

- **Receive**: the UART1 interrupt is permanently on (RC + all error bits). A
  byte arrives → the IRQ drains RDR byte by byte into the 64-byte RX pipe
  (when full, bytes are dropped and counted by a diagnostic counter); on the
  task side `rx.read(&mut buf).await` takes data from the pipe, suspending
  until at least 1 byte is available.
- **Transmit**: `tx.write(&buf[..n]).await` writes the data into the 64-byte
  TX pipe and returns (suspending only when the pipe is full), and kicks the
  background drain once — the IRQ moves pipe contents into TDR whenever TXBE
  allows (clearing TC before writing, so a stale flag cannot wake `flush`
  early); `flush()` waits for "pipe empty + TC set", i.e. the last byte has
  actually finished shifting out.

The example runs echo and heartbeat as two concurrent tasks: the echo task is
parked on `rx.read`, the heartbeat task toggles PC13 every second — as long as
the LED keeps blinking, the executor is alive and echo is merely stopped at
the pipe read.

```rust
//! Chapter 8 example 2: BufferedUart echo + heartbeat LED (two-task concurrency).
//!
//! UART1 (PA02=TXD / PA03=RXD, AF1, 115200-8N1) running BufferedUart:
//! receive goes into a 64-byte ring buffer (embassy-sync Pipe), the echo task
//! echoes it back, the heartbeat task toggles PC13 (1-second period, the
//! observable "system is alive" signal).
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch08_uart_buffered -p cw32-book-embassy
//! Status: verified via the same-source example (embassy-cw32 examples/buffered_echo)
//!       (2026-10-24: heartbeat LED + ISER bit27 + injected pending, full chain;
//!       data loopback awaits a USB-to-serial test, source in the Chapter 8 text).

#![no_std]
#![no_main]

use embassy_executor::{main, task};
use embassy_time::Timer;

use cw32_book_embassy::board;
use embassy_cw32::{
    Config, Peripherals,
    gpio::Pin,
    uart::{self, BufferedUart},
};

embassy_cw32::bind_interrupts!(struct Irqs {
    UART1 => uart::Uart1Irq;
});

#[task]
async fn echo(
    mut rx: embassy_cw32::uart::BufferedUartRx,
    mut tx: embassy_cw32::uart::BufferedUartTx,
) {
    let mut buf = [0u8; 32];
    loop {
        let n = rx.read(&mut buf).await; // suspend until at least 1 byte is in the pipe
        tx.write(&buf[..n]).await; // into the TX pipe; kicks the background drain
        tx.flush().await; // wait for TC: the last byte has truly finished shifting
    }
}

#[main]
async fn main(spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default());

    let p = Peripherals::take().unwrap();
    // PC13 heartbeat LED: gating + construction live in board::led() (the crate lib's board wrapper)
    let mut led = board::led();

    let u = BufferedUart::new_uart1(
        p.uart1,
        Pin::new(p.gpioa, 2),
        Pin::new(p.gpioa, 3),
        1, // AF1
        Irqs,
        Default::default(), // uart::Config: 115200
    );
    let (tx, rx) = u.split();
    spawner.spawn(echo(rx, tx).unwrap());

    loop {
        Timer::after_millis(1000).await;
        led.toggle();
    }
}
```

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch08_uart_buffered -p cw32-book-embassy
```

The pinout follows the user board: UART1 = PA02(TXD)/PA03(RXD), alternate
function AF1 (manual Table 9-2; the measured wiring is in Chapter 5).
`Default::default()` is a `uart::Config` with the default baud rate
115200-8N1.

`rx.read(&mut buf)` returns a `usize` rather than a `Result` — an inherent
method of `BufferedUartRx` (blocks until at least 1 byte, returns the number
actually read). The reason it can return "without errors" is that receive
errors are already dealt with at the IRQ layer: bad bytes are dropped and
counted, never entering the pipe. There are two diagnostic handles:
`rx.ready()` reports how many bytes are currently in the pipe, `rx_dropped()`
the running total of dropped ones.

## 8.4 Byte-by-Byte Uart vs BufferedUart

The embassy-cw32 UART has two task-side forms (the L012 also has a third, DMA
path; its semantic-level implementation still awaits on-board validation and
is covered in Chapter 12; the three modes are mutually exclusive on one
instance):

| | Byte-by-byte `Uart` (Chapter 9 example) | `BufferedUart` (Example 2 of this chapter) |
|---|---|---|
| Receive path | IRQ wakes the task, the task reads RDR itself (1 byte) | IRQ drains RDR into a 64-byte pipe, the task reads the pipe |
| Burst tolerance | No buffering: the task must read before the next byte arrives (about 87µs at 115200), or it overruns (ORE) | The task can be about 5.6ms late (64 bytes @115200) without losing data |
| Transmit pacing | `write_byte` waits for TXBE, advancing byte by byte | `write` returns once the data is in the pipe; the IRQ takes over pacing; `flush` explicitly waits for TC |
| Multiple tasks | After `split`, one task each for receive and transmit | The rx/tx halves can belong to different tasks; the read end is `Pipe`-based and naturally supports concurrent readers |
| Error reporting | `read_byte` returns `UartError` (ORE/FE/PE/NE, in that priority order) | Error bytes are dropped and counted at the IRQ layer (`rx_dropped()`), never reported to the task |
| RAM cost | No static buffers | 64 bytes RX + 64 bytes TX per instance (the pipes of 3 instances are global statics, 384 bytes total) |

The selection rule in one sentence: **for imperative, low-rate interactions
where you want to inspect errors byte by byte, use the byte-by-byte `Uart`;
for concurrency, burst tolerance, and steady throughput (log output, protocol
streams), use `BufferedUart`**. The byte-by-byte form also carries a hidden
requirement: the main task must spend most of its time "waiting on the serial
port" — if, after being woken, it wanders off to do 100µs of something else,
the next byte may already be lost. `BufferedUart` shifts that timing pressure
from the task to the interrupt (the interrupt only needs to drain RDR within
one byte time).

## 8.5 Build and On-Hardware Verification

Example 2 is labeled **verified via the same-source example**, sourced from
the 2026-10-24 entry in the "Verification status" section of the embassy-cw32
README — `buffered_echo` (the direct same-source relative of this example) on
a CW32L012 board:

- The heartbeat LED toggles continuously, proving the executor is running with
  two concurrent tasks (the echo task parked at the pipe read);
- `ISER bit27 = 1` (the NVIC line of UART1), proving the v0.2 fix
  "constructors automatically enable NVIC" works;
- With a UART1 pending injected into `ISPR` through the debug probe, the
  interrupt walked the full chain normally: vector → `bind_interrupts!` →
  `buffered_irq` → critical-section; `ICPR` cleared, no lockup;
- **Honest labeling**: the actual buffered data loopback (what the PC sends,
  what the board returns) was not measured at the time — no USB-to-serial
  adapter was on hand. For the first complete on-board run, check: crossed
  adapter wiring (adapter RXD to board PA02), 115200-8N1, and a byte-for-byte
  echo of whatever you send.

Example 1 (`ch08_gpio_wait`) is labeled **compiles, not yet run on hardware**.
First-run checklist:

- If the board has no button, touch PA0 to GND with a jumper wire; each touch
  toggles the LED once;
- Watch whether one "press" corresponds to several toggles (bounce; see the
  debounce exercise in 8.2);
- When changing pins, three places must stay consistent: the pin number in
  `Input::new`, the port name in `bind_interrupts!`, and the AHBEN bit
  (GPIOA/B/C = bit4/5/6).

## 8.6 Common Pitfalls

- **Forgetting to bind the port interrupt**: `wait_for_*` suspends forever —
  the vector lands in the weak default `DefaultHandler` (an endless loop),
  no error. Bind whichever port you wait on.
- **Port clock not enabled (or overwritten)**: before `Input`/`Output` touch
  registers, confirm the AHBEN bit is set. Two ports must be enabled in one
  keyed write; two writes overwrite each other.
- **Button bounce**: several toggles per press is not a bug, it is physics.
  Add a 50ms level re-check.
- **No echo on the serial port**: first check AF1 and the crossed wiring (the
  board's TXD to the adapter's RXD), then check that `bind_interrupts!` bound
  UART1.
- **Assuming BufferedUart never loses data**: the 64-byte pipe is a hard
  bound; once RX is full, new bytes are dropped and counted in
  `rx_dropped()`. For bulk transfers add your own flow control, or move to
  DMA (Chapter 12).
- **Mixing the three transfer modes**: byte-by-byte / Buffered pipe / DMA are
  mutually exclusive on one UART instance; let the previous mode wind down
  before switching (the `uart` module docs).
