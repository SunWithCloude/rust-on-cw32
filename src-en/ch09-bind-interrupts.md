# Chapter 9 bind_interrupts! and Interrupt Binding

**In this chapter you will learn:**

- What the `bind_interrupts!` macro actually expands to: a strong-symbol
  vector function + a compile-time "binding proof";
- Why binding the wrong interrupt is a compile error rather than a runtime
  accident (real errors from two real experiments);
- The respective roles of the two types/traits `IrqHandler` and `Binding`;
- Where the NVIC enable happens — and the ecosystem repo's v0.1 lesson of
  "vector present, but interrupts never arrive";
- The minimal example `ch09_bind_uart.rs`.

Chapter 8 already used `bind_interrupts!` twice: binding the GPIOA port
interrupt, and binding UART1. This chapter takes the macro apart and looks at
every part.

## 9.1 The Problem: How an Interrupt Vector Reaches Your Code

At the hardware level, after reset the Cortex-M0+ fetches the entry address of
every exception/interrupt from the vector table (`__INTERRUPTS`, laid down by
cortex-m-rt). The CW32L012 has 32 peripheral interrupt slots; the PAC's `rt`
feature generates one weak alias per slot in its build script
(`cw32l012-pac` `build.rs`):

```text
PROVIDE(GPIOA = DefaultHandler);
PROVIDE(UART1 = DefaultHandler);
… (32 lines in total; the slot number is the IRQ number: GPIOA=5, GPIOB=6,
  GPIOC=7, GPIOF=8, UART1=27, UART2=28, UART3=29, …)
```

The semantics of `PROVIDE` are: "if nobody defines this symbol, fall back to
`DefaultHandler`". So the C-style way to bind an interrupt is to **define a
strong symbol of the same name** to cover it:

```rust,ignore
#[unsafe(no_mangle)]
unsafe extern "C" fn UART1() {
    // your handler code
}
```

The name comes from the vector table; misspell it by one letter (say
`UART_1`) and the linker **will not complain** — the weak alias keeps serving
as the fallback, your function becomes dead code nobody calls, and interrupts
silently land in the `DefaultHandler` endless loop. Chapter 2 planted this
seed: what the PAC pulls into linking is exactly this vector-table skeleton;
nothing went wrong back then only because the examples did not yet need
peripheral interrupts.

What `bind_interrupts!` solves is precisely the problem of "is the name
right" quietly slipping past link time.

## 9.2 Macro Expansion: One Macro, Two Things

```rust,ignore
embassy_cw32::bind_interrupts!(struct Irqs {
    UART1 => uart::Uart1Irq;
});
```

This macro line expands (`embassy-cw32/src/interrupt.rs`) into two things:

**First, the vector function.** A strong symbol is generated for `UART1`,
overriding the PAC's weak alias:

```rust,ignore
#[unsafe(no_mangle)]
unsafe extern "C" fn UART1() {
    <Uart1Irq as IrqHandler<interrupt::UART1>>::call()
}
```

`uart::Uart1Irq` is the driver-provided "one per IRQ" marker type, and it
implements the `IrqHandler<UART1>` trait (its `fn call()` runs the driver's
own dispatch logic in interrupt context — the Chapter 8 `buffered_irq` that
drains the pipe and the `on_irq` that wakes wakers both enter here). The
vector function itself carries no business logic; it only hands control to
the driver's handler type.

**Second, the binding proof.** A marker trait implemented for your struct:

```rust,ignore
unsafe impl Binding<interrupt::UART1> for Irqs {}
```

`Binding<INT>` is an empty trait (it is `unsafe` because the fact that "this
struct really is bound to this interrupt" can only be guaranteed by the
macro). It carries no data; it merely writes "Irqs holds the UART1 interrupt"
into the type system — for the constructor in the next step to check.

The two are two sides of one coin: the vector function guarantees someone
answers when the interrupt arrives, and `Binding` guarantees that to use the
peripheral you must first have bound its vector.

## 9.3 Compile-Time Checking in the Constructor

The driver's constructor puts the proof into its signature
(`embassy-cw32/src/uart.rs`):

```rust,ignore
pub fn new_uart1(
    regs: &'static crate::Uart,
    tx: Pin<impl GpioPort>,
    rx: Pin<impl GpioPort>,
    af: u8,
    _irqs: impl Binding<interrupt::UART1>,   // ← the check point
    config: Config,
) -> Self
```

The parameter name carries an underscore (`_irqs`) — it is never used at
runtime; its sole reason to exist is to let the compiler check the type.
**Before constructing the peripheral, its interrupt must already be bound**,
or the build fails.

What happens if you bind wrong? The two experiments below are real compile
results (`thumbv6m-none-eabi` release, verbatim excerpts of the rustc
errors). The experiment files themselves never enter `examples/` — their
entire reason to exist is to fail compilation.

**Experiment 1: the interrupt name is right, the handler type is a
mismatch** — binding the GPIO handler to the UART1 interrupt name:

```rust,ignore
embassy_cw32::bind_interrupts!(struct Irqs {
    UART1 => embassy_cw32::gpio::GpioAIrq;   // wrong: this is GPIOA's handler
});
```

The real error:

```text
error[E0277]: the trait bound `GpioAIrq: IrqHandler<UART1>` is not satisfied
   --> examples/embassy/examples/zz_exp1_wrong_handler.rs:16:14
    |
16  |     UART1 => embassy_cw32::gpio::GpioAIrq;
    |              ^^^^^^^^^^^^^^^^^^^^^^^^^^^^ the trait `IrqHandler<UART1>` is not implemented for `GpioAIrq`
    |
help: the trait `IrqHandler<UART1>` is not implemented for `GpioAIrq`
      but trait `IrqHandler<GPIOA>` is implemented for it
    = help: for that trait implementation, expected `GPIOA`, found `UART1`
```

The macro intercepts the error right at the expansion site: `GpioAIrq`
implements only `IrqHandler<GPIOA>` and cannot impersonate UART1's handler.
Contrast with the C style — the linker says nothing about this class of
mistake.

**Experiment 2: the binding itself is legal, but the proof the constructor
wants is missing** — GPIOA is bound perfectly well, yet used to construct a
UART:

```rust,ignore
embassy_cw32::bind_interrupts!(struct Irqs {
    GPIOA => gpio::GpioAIrq;   // legal, but unrelated to UART
});
// …
let mut u = Uart::new_uart1(p.uart1, tx, rx, 1, Irqs, Default::default());
```

The real error:

```text
error[E0277]: the trait bound `Irqs: Binding<UART1>` is not satisfied
   --> examples/embassy/examples/zz_exp2_missing_binding.rs:28:9
    |
23  |     let mut u = Uart::new_uart1(
    |                 --------------- required by a bound introduced by this call
28  |         Irqs,
    |         ^^^^ unsatisfied trait bound
    |
help: the trait `Binding<UART1>` is not implemented for `Irqs`
      but trait `Binding<GPIOA>` is implemented for it
note: required by a bound in `embassy_cw32::uart::Uart::new_uart1`
324  |         _irqs: impl Binding<interrupt::UART1>,
    |                     ^^^^^^^^^^^^^^^^^^^^^^^^^ required by this bound
```

The error even suggests your next step: add `UART1 => uart::Uart1Irq;` to
`Irqs`.

Together the two experiments say one thing: **the interrupt number and the
handler type are welded together at compile time** — firmware that binds the
wrong interrupt simply cannot exist.

## 9.4 Where the NVIC Enable Happens

`bind_interrupts!` generates only the vector and the proof; it **never
touches the NVIC**. The enable happens inside the driver constructors (and
GPIO's `wait_edge`): `nvic_enable::<I>()` writes ISER directly (0xE000_E100,
`1 << I::NUMBER`; IRQ27 is bit27).

This division of labor once caused a real incident worth learning from (the
0.2.0 "fixed" entry of the embassy-cw32 CHANGELOG): in v0.1 every async
driver had its vector function and its `Binding` — **but nobody wrote ISER**,
so interrupts never got in at all. The bug stayed hidden for an interesting
reason: the blink example's timing went through SysTick, which is a core
exception and never crosses the NVIC's peripheral interrupt lines; the
interrupt-driven path was first truly exercised only when the buffered UART
was ported. After the fix, constructing `Uart`/`BufferedUart`/`Spi`/`I2c` and
GPIO's `wait_edge` all automatically enable the corresponding NVIC line — the
on-board evidence is the `buffered_echo` record quoted in Chapter 8:
`ISER bit27 = 1`.

Design-wise, "enable at construction" is sound: the moment you hold a driver
object is the moment it starts producing interrupts. Only one thing to keep
in mind — an unbound IRQ falls into the `DefaultHandler` endless loop, the
runtime tombstone of "whoever forgot to bind the interrupt"; an IRQ that is
bound but unused by any driver has a vector function and no NVIC enable,
hence no side effects.

## 9.5 Example: ch09_bind_uart.rs

The minimal closed loop: one `bind_interrupts!` + a byte-by-byte `Uart` echo
(UART1, PA02/PA03 AF1, 115200-8N1).

```rust
//! Chapter 9 example: minimal UART use of bind_interrupts! (byte-by-byte echo).
//!
//! UART1 (PA02=TXD / PA03=RXD, AF1, 115200-8N1), byte-by-byte Uart:
//! read_byte waits for RC, write_byte waits for TXBE, and the task suspends
//! while waiting. The constructor requires a proof of `impl Binding<UART1>`
//! — Irqs is generated by bind_interrupts!, and binding the wrong interrupt
//! (say, writing GPIOA where UART1 belongs) fails the build outright
//! (Chapter 9 shows the real errors from those experiments).
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch09_bind_uart -p cw32-book-embassy
//! Status: verified via the same-source example (embassy-cw32 examples/uart_echo_async),
//!       run on hardware: it runs, no hard fault (README verification record);
//!       the full rx/tx loopback awaits a USB-to-serial test — source and
//!       checklist in the Chapter 9 text.

#![no_std]
#![no_main]

use embassy_executor::main;

use cw32_book_embassy as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use embassy_cw32::{
    Config, Peripherals,
    gpio::Pin,
    uart::{self, Uart},
};

// Bind the UART1 interrupt (IRQ27) to the driver's Uart1Irq handler type: the
// macro expands into the strong-symbol vector function UART1 (overriding the
// weak default in the PAC's device.x) and implements Binding<UART1> for Irqs
embassy_cw32::bind_interrupts!(struct Irqs {
    UART1 => uart::Uart1Irq;
});

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default());

    let p = Peripherals::take().unwrap();

    // Construction doubles as validation: Irqs must implement
    // Binding<interrupt::UART1> or the build fails. The NVIC line (ISER
    // bit27) is enabled automatically by the constructor
    let mut u = Uart::new_uart1(
        p.uart1,
        Pin::new(p.gpioa, 2),
        Pin::new(p.gpioa, 3),
        1, // AF1
        Irqs,
        Default::default(),
    );

    loop {
        match u.read_byte().await {
            Ok(b) => {
                u.write_byte(b).await;
            }
            Err(_e) => {} // the error flags were already cleared via ICR inside read_byte; keep receiving
        }
    }
}
```

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch09_bind_uart -p cw32-book-embassy
```

The echo body is the minimal version of the "byte-by-byte `Uart`" column from
Chapter 8: `read_byte().await` suspends on RC (the RC bit of IER is opened
only while waiting — no interrupt storm while idle), and once a byte arrives,
`write_byte().await` waits for TXBE and writes TDR. `Err(_e) => {}` is not
swallowing errors — before `read_byte` returns the error, it has already
cleared the error flags via ICR (ICR is R1W0, write 0 to clear; manual §21,
UART register group), so here we simply keep receiving the next byte; if you
do want to distinguish error kinds, the four `UartError` variants
(Overrun/Framing/Parity/Noise) are a `match` away.

For the division of labor with the `BufferedUart` version, see the comparison
table in 8.4: this example demonstrates the "byte-by-byte" form, and both
forms share the same `bind_interrupts!` and constructor validation.

## 9.6 Verification on Hardware

This example shares its source with the ecosystem repo's `uart_echo_async`
(comments aside), whose entry in the embassy-cw32 README "Verification
status" reads (2026-10-24): **flashed to the board, runs, no hardware fault;
the actual rx/tx of the async loopback could not be measured at the time — no
USB-to-serial adapter was available**. This example is therefore labeled:
**verified via the same-source example (limited)** — running on hardware is
on record, the full data loopback still awaits measurement. First complete
on-board checklist:

- Crossed adapter wiring (adapter RXD to board PA02/TXD), 115200-8N1;
- Byte-by-byte echo: send any sequence, the echo matches byte for byte;
- Deliberately send a byte stream at a mismatched baud rate to create framing
  errors, and confirm the firmware does not lock up (the `Err` branch keeps
  receiving bytes).

## 9.7 Common Pitfalls

- **Bound the interrupt but forgot to pass `Irqs`** (or passed a different
  struct): E0277, `Binding<UART1>` not satisfied — fix it following the
  Experiment 2 error in 9.3.
- **Binding the same interrupt twice**: an interrupt may appear only once in
  one set of `Binding`s; writing it twice generates duplicate vector symbols
  and duplicate trait impls, and the build fails outright.
- **Writing your own `#[unsafe(no_mangle)] fn UART1` on top**: duplicate
  definition of the strong symbol the macro generated — a link error. Put
  interrupt logic into the driver's handler type, or provide your own
  `IrqHandler` implementation — do not mint a second vector symbol.
- **The "bound but not enabled" misconception**: the NVIC enable happens
  automatically in the constructor/`wait_edge`; you do not need to (and
  should not) write ISER by hand. If you insist on doing it manually, first
  be sure what exactly you are bypassing.
- **Shared-vector families**: SPI2/3 share the `SPI23` interrupt, and on the
  L083 UART1 and UART4 share a vector (typelevel name `UART1_UART4`). Not
  needed on the L012; for cross-family ports see the family matrix in
  Appendix A.
