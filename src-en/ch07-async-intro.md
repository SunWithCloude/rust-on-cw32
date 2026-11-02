# Chapter 7 Into the Async World

**In this chapter you will learn:**

- The essential CPU-level difference between the blocking `Delay` and the async
  `Timer::after(...).await`;
- How the embassy executor organizes tasks with `async fn` (`#[main]`,
  `#[task]`, `Spawner::spawn`);
- Why the embassy-cw32 SysTick time driver is a "dynamic one-shot alarm", and
  what the CPU does while idle;
- What role `Config`/`init()` plays in async firmware;
- A segment-by-segment close reading of `ch07_blink_async.rs`, flashing it to
  the board, and seeing PC13 blink with precision.

## 7.1 From Spinning to Suspending

When Part 2 blinked an LED, the main loop contained one line:
`delay.delay_ms(500)`. The `delay` module of `cw32l012-hal` describes itself as
a **SysTick busy-wait delay** (`cw32l012-hal` `src/delay.rs`): it configures
SysTick as a free-running down-counter, then repeatedly reads `COUNTFLAG` in a
loop until enough ticks have elapsed. During those 500 ms the CPU spends 100%
of its time inside that polling loop — even switching to a timer interrupt
plus a polled flag is still, at heart, "holding the CPU while waiting".

With a single task this is fine. To do two things at once (blink an LED + scan
a keypad), you either hand-write a state machine that chops each task into
small steps executed in turn by the main loop, or you bring in an RTOS and pay
for task switching to keep the code looking sequential.

Async offers a third way, and its core promise is one sentence: **while
waiting, give the CPU away**.

```rust,ignore
Timer::after_millis(500).await;
```

This line does not poll in a loop. What it does: register the "deadline 500 ms
from now" in the timer queue, then tell the executor "do not schedule me again
until then" — the current task suspends at the `.await` point, and when the
executor finds no other runnable task it puts the core to sleep (the WFE
instruction). After 500 ms the timer interrupt fires, wakes the task, and
execution continues at `led.toggle()`. While waiting, CPU usage is zero.

`async`/`await` is only a language mechanism: an `async fn` compiles into a
state machine (implementing `Future`), and each `.await` point is a place
where the state machine can yield. What actually decides "who gets scheduled
when" is the **executor**. The Rust standard library ships no executor — which
is exactly where embassy enters.

## 7.2 The embassy Executor: Tasks Are Futures

embassy is the de facto standard async runtime of the embedded-wg ecosystem.
For this book you only need three building blocks: `#[main]`, tasks, and the
`Spawner`.

**`#[main]`.** Placed on the async entry function; after macro expansion it
generates the real entry point (the cortex-m-rt entry): it creates an executor
instance, mounts your async main as the first task, and enters the scheduling
loop. The idle policy of embassy-executor on Cortex-M is written in its source:
with no runnable task it executes `asm!("wfe")` and sleeps, waking when an
interrupt arrives (embassy-executor 0.10 `src/platform/cortex_m.rs`). This is
the last piece of the "waiting costs no CPU" puzzle — not only is the task
waiting, the whole core is asleep.

**Tasks.** A task is an `async fn` annotated with
`#[embassy_executor::task]`. The macro rewrites it into a plain function that
returns `Result<SpawnToken<impl Sized>, SpawnError>`
(embassy-executor-macros 0.8 `src/macros/task.rs`) — calling it merely "picks
up a task token"; handing the task to the executor is the next step:

```rust,ignore
spawner.spawn(echo(rx, tx).unwrap());
```

The `unwrap()` handles "token acquisition failure": by default only one
instance of a task may run at a time (the task's static storage exists exactly
once), and spawning again yields `SpawnError::Busy` (if you need multiple
instances, add `pool_size = N` to `#[task]`, at the cost of one RAM copy per
instance). `spawner.spawn(token)` puts the task on the executor's queue,
waiting for its first poll.

**Ownership.** Tasks are `'static` — the executor has no idea when your task
will end, so arguments passed into a task must be the values themselves (pin
halves, `BufferedUartRx`, and so on), not references to stack variables. That
is why the later examples move the two **ownership halves** `(tx, rx)` of
`u.split()` straight into the `echo` task.

Waiting inside a task relies on wakers: at the `.await` point the task
registers its waker with the "event provider" (the time driver's timer queue,
the UART interrupt's waker table, the GPIO per-pin waker table) and returns
`Pending`; when the interrupt fires, the event provider calls `wake()`, and
the executor polls the task again in the next scheduling round. Chapter 8
shows how this machinery is wired up for GPIO/UART.

## 7.3 The SysTick Time Driver: a Dynamic One-Shot Alarm

That `Timer::after_millis(500)` wakes you exactly on time is the work of the
time driver built into embassy-cw32. The crate's own library documentation
sums it up (header comment of `src/lib.rs`):

> `init`: SysTick time driver (1MHz tick, dynamic one-shot alarm, no wakeups
> while idle)

Taking it apart:

**The embassy-time-driver 0.2 interface.** embassy-time only provides types
such as `Instant`/`Timer`; the real "what time is it" and "call me when it is
time" are implemented by the time driver — a `Driver` trait with just two
methods: `now()` returns the current instant, `schedule_wake(at, waker)`
registers a deadline wakeup. embassy-cw32 fixes the tick at 1MHz with
`tick-hz-1_000_000`, so the counting unit of `Timer` is the microsecond
(`embassy-cw32/src/time_driver.rs`).

**The M0+ SysTick has no compare register.** The Cortex-M0+ SysTick is a
24-bit down-counter (ARMv6-M Architecture Reference Manual, DDI 0419 §B3.3):
on reaching 0 it sets `COUNTFLAG`/raises an interrupt, reloads `RVR`, and
starts over. By nature it can only do "periodic interrupts", never "interrupt
at a specified instant".

**The dynamic one-shot alarm** is aimed squarely at this limitation: most of
the time SysTick free-runs at its maximum period (2^24 systicks); whenever the
**head deadline of the timer queue changes**, the driver reprograms `RVR` to
"how many systicks remain until the earliest deadline" and writes `CVR = 0` to
restart the count — a one-shot alarm. In the expiry interrupt the due timers
are dispatched and the next alarm is scheduled (`time_driver.rs` module docs
and `set_alarm()`).

**No wakeups while idle.** A traditional 1ms tick interrupt barges into the
CPU every millisecond whether or not any timer is running. Here there is no
fixed tick: when the queue is empty there is no "next deadline" to chase,
SysTick degenerates to free-running at maximum period, and the only periodic
event is the wrap of the 24-bit counter (about every 4.19 s at 4MHz HCLK); the
interrupt does only bookkeeping and wakes no task. For low-power scenarios,
this is what "no periodic wakeups while idle" means.

Three boundary behaviors worth knowing when you write code:

- **Long-deadline degradation**: when a deadline exceeds 24 bits (about 4.19 s
  at 4MHz), the driver first programs the maximum period and re-checks the
  queue in the wrap interrupt — `Timer::after_secs(10)` is still exact, it
  just gets a few extra bookkeeping interrupts in between.
- **Never early, possibly late**: an alarm closer than `MIN_PERIOD` (100
  systicks, 25µs at 4MHz) can fire up to that many systicks late, never early
  — firing early would break embassy-time's semantics, and the lateness is
  bounded (the `MIN_PERIOD` comment in `time_driver.rs`).
- **now() is monotonic and never goes backwards**: the 64-bit software
  extension is computed via a read-and-clear of `COUNTFLAG`, with a
  double-read check closing the wrap window; the first reading after a
  reprogram briefly runs ahead and is clamped back by the monotonic clamp
  (`now_systicks()`). This timing logic once had a real bug — the
  reprogramming base was mistakenly stored as `now - (period - 1)`, which made
  `now()` oscillate within one period, deadlines were never reached, and the
  LED stayed lit without blinking; it was diagnosed with probe-rs register and
  RAM reads and then fixed (hardware debugging note in the embassy-cw32 README
  "How the time driver works", 2026-10-24). The point of quoting it here:
  every detail of this code is backed by on-hardware debugging.

## 7.4 Config and init()

```rust,ignore
embassy_cw32::init(Config::default());
```

embassy-cw32 **does not configure the clock tree**. `Config { hclk_hz,
pclk_hz }` merely *declares* the current bus frequencies to the library: the
SysTick systick→microsecond conversion and the UART baud-rate division both
depend on it. `Config::default()` follows the reset defaults — 4MHz for the
L012 generation (HSI 96MHz / 24; the divider encoding is in manual §4.7.4),
HCLK = PCLK = 4MHz. If your firmware changes the clock, configure it at the
register level first, then call `init()` with the actual frequencies.

`init()` starts the SysTick time driver and must be the first line of the
`#[main]` function — it has to be in place before any time-dependent task
starts running.

Another global constraint lives at this layer too: embassy-cw32 enables a
PRIMASK-based critical-section implementation by default, and **a firmware can
contain only one critical-section implementation** — do not mix the Part 2
`cw32l012-hal` (blocking HAL) and `embassy-cw32` into the same firmware;
linking will conflict. This book isolates them at the dependency level by
giving each part its own example crate (`examples/pac`, `examples/hal`,
`examples/embassy`).

## 7.5 A Close Reading of ch07_blink_async.rs

The whole example is under 40 lines; here it is in full
(`examples/embassy/examples/ch07_blink_async.rs`):

```rust
//! Chapter 7 example: async GPIO blink (PC13, the usual on-board LED pin).
//!
//! Minimal closed loop of the embassy executor + SysTick time driver + async
//! tasks: `Timer::after_millis(500)` merely puts the task back onto the timer
//! queue, and the SysTick alarm wakes it when the deadline arrives — while
//! waiting, the CPU sleeps in the executor (WFE) instead of spinning.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch07_blink_async -p cw32-book-embassy
//! Status: verified via the same-source example (embassy-cw32 examples/blink_async)
//!       (2026-10-24: PC13 toggles at precise 500ms intervals; measured SysTick
//!       alarm period 1,997,499 ≈ 2,000,000 systicks, source in the Chapter 7 text).

#![no_std]
#![no_main]

use embassy_executor::main;
use embassy_time::Timer;

use cw32_book_embassy as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use embassy_cw32::{
    Config, Peripherals,
    gpio::{Level, Output, Pin},
    sysctrl_enable_gpioc,
};

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // reset-default SysClk = 4MHz

    let p = Peripherals::take().unwrap();
    sysctrl_enable_gpioc(p.sysctrl);

    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    loop {
        Timer::after_millis(500).await;
        led.toggle();
    }
}
```

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch07_blink_async -p cw32-book-embassy
```

Segment by segment:

**The first lines: attributes and where the panic handler lives.**
`#![no_std]`/`#![no_main]` were explained one by one in Chapter 2. There is no
`#[panic_handler]` visible in this example — it is defined at the root of this
example crate's `lib.rs`, and `use cw32_book_embassy as _;` pulls the lib into
linking, inheriting it automatically. This is exactly the panic-probe
mechanism: an entire crate can be nothing but a panic handler, effective as
soon as it is imported with `as _` (the Chapter 2 examples still carry their
own handler for teaching purposes; from the async part on, the boilerplate
gives way to the subject matter).

**init.** `embassy_cw32::init(Config::default())` starts the SysTick time
driver at the 4MHz reset-default clock. It is the first line of the `#[main]`
function; Section 7.4 explains why.

**Peripherals::take()**. The PAC-generated peripheral singletons; after one
successful `take()` it returns `None` — guaranteeing a single owner for every
peripheral register block. The `unwrap()` cannot blow up here: each core gets
exactly one chance.

**GPIOC clock gating.**

```rust,ignore
sysctrl_enable_gpioc(p.sysctrl);
```

The SYSCTRL gating registers of the L012 generation are write-protected: the
upper 16 bits of every write must carry the key `0x5A5A`, otherwise the entire
write is discarded (manual §4.7, SYSCTRL register group). What
`sysctrl_enable_gpioc` does is write `SYSCTRL_KEY | (1 << 6)` (the GPIOC bit
of AHBEN) in one go.

Here hides this chapter's most important, most easily tripped-over trap:
**AHBEN is a key-protected whole-word write**. The key occupies the upper 16
bits of the register and the enable bits the lower 16, so every write covers
the whole register. Call `sysctrl_enable_gpioa()` and then
`sysctrl_enable_gpioc()`, and the latter wipes out the GPIOA bit the former
just set. To enable two ports you must pack both bits into **the same write**
— Chapter 8's `ch08_gpio_wait` does exactly that.

**Lighting the LED.** `Output::new(Pin::new(p.gpioc, 13), Level::High)`:
`Pin::new` binds the port and pin number; `Output::new` clears the pin's
`GPIOx_ANALOG` bit, sets the direction to output, and applies the initial
level through BSRR/BRR. The ANALOG step is a genuine L012 trap: **after reset
PC13 is in analog mode (`GPIOx_ANALOG.PIN13 = 1`)**, which cuts off all
digital input/output — unless this bit is cleared, the LED stays dark. The
Chapter 4 blocking HAL already handled the same problem (header comment of
`cw32l012-hal` `examples/blink_pc13.rs`).

**The main loop.**

```rust,ignore
loop {
    Timer::after_millis(500).await;
    led.toggle();
}
```

Each iteration: `Timer::after_millis(500)` registers a deadline 500 ms out in
the timer queue and the task suspends; the executor sleeps in WFE; 500 ms
later the SysTick alarm interrupt dispatches and wakes it, and `toggle()` flips
the pin atomically through the TOG register (Chapter 4). Do the arithmetic:
500ms = 2,000,000 systicks (4MHz), within the 24-bit limit (16,777,216), so a
single programming step suffices — this is the number to be measured on the
board below.

## 7.6 Flashing and Observing

```bash
probe-rs download --chip CW32L012x8 \
  target/thumbv6m-none-eabi/release/examples/ch07_blink_async
probe-rs reset --chip CW32L012x8
```

The chip name `CW32L012x8` is recognized out of the box by probe-rs 0.33+
(Appendix D is a probe-rs cheat sheet). Without a logic analyzer, verify by
eye: roughly half a second on, half a second off, a 1-second period — visually
indistinguishable from the blocking version. The difference is in the CPU:
once this firmware runs, the 4MHz CPU is asleep between two toggles.

## 7.7 On-Hardware Verification

The "Verification status" section of the embassy-cw32 README records the
on-hardware data for this example (2026-10-24, probe-rs + ST-Link V2):

- **blink_async passed on hardware**: PC13 toggles with a clean 1-second
  period (driven by `Timer::after_millis(500)`, confirmed by polling ODR
  through the probe); the measured SysTick alarm period was
  **1,997,499 ≈ 2,000,000 systicks** (500ms @ 4MHz HCLK) — the async executor,
  the time driver, and task wakeup work end to end.

This repo's `ch07_blink_async.rs` is **line-for-line the same source** as that
example (comments aside), so this chapter's example is labeled: **verified on
hardware (via the same-source example)** — the verification data comes from
the on-board record in the ecosystem repo embassy-cw32.

## 7.8 Common Pitfalls

- **Forgetting `init()` or putting it elsewhere**: the time driver never
  starts, every `Timer` mistimes or hangs outright. Keep `init()` fixed as the
  first line of `#[main]`.
- **Not updating `Config` after changing the clock**: the library does not
  configure the clock tree; if you change HCLK but still pass 4MHz, every
  Timer and the baud rate mistime. After a clock change, call `init` again
  with the actual frequencies.
- **PC13 will not light**: reset-default analog mode
  (`GPIOx_ANALOG.PIN13=1`) plus an un-gated GPIOC. In this example
  `Output::new`/`sysctrl_enable_gpioc` handle both for you — when you switch
  to another pin, remember to check the same two steps.
- **Enabling two port clocks, one overwriting the other**: the
  `sysctrl_enable_gpioX` family is a key-protected whole-word write (upper 16
  bits must be 0x5A5A); the second write clears the first one's bits. Enable
  both ports in a single write (demonstrated in the Chapter 8 example).
- **Mixing two critical-section implementations**: `embassy-cw32` (enabled by
  default) and `cw32l012-hal`'s `critical-section-impl` cannot coexist in one
  firmware; the link fails. This book isolates them with separate example
  crates.
- **Very short timers fire late**: alarms closer than 100 systicks (25µs at
  4MHz) may be that much late, never early. Irrelevant for millisecond-scale
  periods.
