# Chapter 12 DMA

In this chapter you will learn:

- The L012 DMA's 4-channel model and its "block × block size" transfer
  counting (§8);
- The difference between software triggers and hardware triggers (the `hs`
  trigger-source Table 8-8);
- How to write the minimal `Dma::mem_to_mem` loop, plus four
  hardware-verified pitfalls;
- What the UART/SPI/ADC/DAC DMA variants look like (pointers to their
  sections; not re-expanded here).

## The Channel Model

The L012's DMA controller (§8, base address 0x4002_0000) has **4 channels**,
laid out 0x20 bytes apart within one register block, each channel with its
own CSR/CNT/SRC/DST/TRIG/CCNT. Lower numbers have higher priority
(§8.5.4). The things one transfer needs configured are steps 1~14 of
§8.4.1, which map onto the driver's `Config` fields:

- **Mode** (CSR.TRANS, Table 8-1): BULK moves everything in one burst
  without yielding the bus; BLOCK inserts a gap after each block, letting
  the CPU/higher-priority channels preempt;
- **Trigger** (TRIG.TYPE + TRIG.HARDSRC): a software trigger starts when
  `Channel::start()` writes SOFTSRC; a hardware trigger starts from a
  peripheral event, with the source encoding in **Table 8-8** — the driver
  wraps the whole table as `pac::dma::hs` constants (`UART1_RX=0x00`,
  `ADC1_SEQ=0x0C`, `DAC1=0x10`, `GTIM1_UPD=0x1A`, …);
- **Width** (CSR.SIZE): 8/16/32 bits, **which also fixes the address
  increment at 1/2/4 bytes**;
- **Count** (CNT): `CNT` = number of blocks (1..=65535), `REPEAT` = number
  of elements per block (1..=15, 1 recommended).

Two NVIC lines serve the four channels: **IRQ9 `DMA12` (channels 1/2
shared) and IRQ10 `DMA34` (channels 3/4 shared)**. `Channel::wait().await`
waits for TC/TE (transfer complete/error): TCIE/TEIE for this channel are
enabled only while waiting, and dropped immediately once the interrupt
wakes — the anti-storm discipline runs through the whole book. **Both lines
must be bound** to `Dma12Irq`/`Dma34Irq` (enforced at compile time by
`Dma::new`'s `_irqs` parameter).

## mem_to_mem: The Minimal Loop

`Dma::mem_to_mem(&src, &mut dst)` is the shortest possible DMA experience:
it automatically takes **channel 3** (leaving 1/2 for real-time
peripherals), source/destination both incrementing, BULK, software trigger;
the element type decides the width (u8/u16/u32 → 8/16/32 bits). On
completion `wait()` returns `Result<(), DmaError>`, with errors classified
per the STATUS encoding of Table 8-2 (address out of bounds/aborted/source
error/destination error).

The peripheral-direction convenience entries have the same shape:
`periph_to_mem` (peripheral→memory, takes channel 1, source fixed
non-incrementing) and `mem_to_periph` (memory→peripheral, takes channel 2,
destination fixed). They only configure the DMA side and arm it waiting for
requests — "how the peripheral side raises requests" (the ADC's IER.DMAEOS,
the UART's CR2 bits, SPI's CR2.DMARX/DMATX) belongs to each peripheral's
chapter.

## The Example

`examples/embassy/examples/ch12_dma_m2m.rs`: 64 u32s moved from a FLASH
constant table into SRAM (32-bit width, BULK, software trigger), compared
word by word with the result recorded; a fresh round every 500 ms. Note
that every round goes through `mem_to_mem` again — it re-`configure`s
internally; the reason is pitfall 2 below.

```rust
//! Chapter 12 example: DMA memory→memory transfer (64 × u32, zero wiring).
//!
//! 64 u32s move from the FLASH constant table `SRC` to SRAM `DST` (32-bit
//! width, BULK, software trigger, `Dma::mem_to_mem` takes channel 3),
//! compared word by word with the first mismatch index recorded; then a
//! fresh round every 500ms (each round reconfigures — `CNT.REPEAT` is
//! cleared by hardware after each round of transfers, §8.5.2); the ROUNDS
//! counter is the heartbeat of the async wait path.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch12_dma_m2m -p cw32-book-embassy
//! Status: compiles, not yet run on hardware (same state as the embassy
//!       version's dma_mem2mem); the recipe is same-source as the blocking
//!       dma_demo P1 and verified on hardware (P1_OK=64, DST pattern
//!       word-identical, 2026-10-23, see the hal repo's TESTING.md).
//!       First-flash checklist in the Chapter 12 text.

#![no_std]
#![no_main]

use embassy_executor::main;
use embassy_time::Timer;

use cw32_book_embassy as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use embassy_cw32::{Config, Peripherals};

embassy_cw32::bind_interrupts!(struct Irqs {
    DMA12 => embassy_cw32::dma::Dma12Irq;
    DMA34 => embassy_cw32::dma::Dma34Irq;
});

/// Source data (FLASH constant table, check pattern generated at compile time)
static SRC: [u32; 64] = {
    let mut a = [0u32; 64];
    let mut i = 0;
    while i < 64 {
        a[i] = 0xA5A5_0000 ^ ((i as u32) * 0x0001_0101);
        i += 1;
    }
    a
};

/// Destination buffer (DMA-written; probe-rs reads 256 bytes from 0x20000000 to check the pattern)
#[used]
#[unsafe(no_mangle)]
pub static mut DST: [u32; 64] = [0; 64];
/// Number of word-identical elements (should be 64)
#[used]
#[unsafe(no_mangle)]
pub static mut P1_OK: u32 = 0;
/// Index of the first mismatch (0xFFFF_FFFF = all identical)
#[used]
#[unsafe(no_mangle)]
pub static mut P1_BAD: u32 = 0;
/// Error code: 0=none, 1=DMA error (STATUS decoded in `Channel::check`; should not appear normally)
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;
/// Completed rounds (heartbeat: one DMA TC interrupt wake per round)
#[used]
#[unsafe(no_mangle)]
pub static mut ROUNDS: u32 = 0;

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default());
    let p = Peripherals::take().unwrap();
    let mut dma = embassy_cw32::dma::Dma::new(p.dma, Irqs);

    loop {
        // SRC in FLASH, DST in SRAM: the same path as the blocking dma_demo P1
        let dst = unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(DST) as *mut u32, SRC.len())
        };
        match dma.mem_to_mem(&SRC, dst).await {
            Ok(()) => unsafe { ERR = 0 },
            Err(_) => unsafe { ERR = 1 },
        }

        // Word-by-word comparison (the DMA has completed at TC; the single M0+ core has no cache, so reading SRAM directly gives the latest values)
        unsafe {
            let mut ok = 0u32;
            let mut bad = 0xFFFF_FFFFu32;
            for i in 0..SRC.len() {
                if DST[i] == SRC[i] {
                    ok += 1;
                } else if bad == 0xFFFF_FFFF {
                    bad = i as u32;
                }
            }
            P1_OK = ok;
            P1_BAD = bad;
            ROUNDS += 1;
        }

        Timer::after_millis(500).await;
    }
}
```

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch12_dma_m2m -p cw32-book-embassy
```

## Verification on Hardware

**Status: compiles, not yet run on hardware.** This example is same-source
as the `dma_mem2mem` example in the embassy-cw32 repo; the register recipe
matches the blocking `dma_demo` P1 phase item by item — that recipe was
verified on hardware on 2026-10-23 (hal repo TESTING.md: P1_OK=64,
P1_BAD=0xFFFF_FFFF, DST pattern `A5A5_0000 ^ i×0x0101_0101` word-identical),
and the unverified increment is embassy's async wait path.

First-flash checklist:

- `P1_OK=64`, `P1_BAD=0xFFFF_FFFF`, `ERR=0`, and `DST` (256 bytes from SRAM
  0x20000000) word-identical to the SRC pattern;
- `ROUNDS` +1 every 500 ms — one TC interrupt wake per round, proving the
  async wait is not just spinning idle.

## The UART/SPI DMA Variants

DMA's value shows in streaming peripheral I/O. embassy-cw32 already ships
two ready-made variants; the book does not re-expand them and only gives
the entry points:

- **UART**: `Uart::read_dma` / `Uart::write_dma` (a single full-volume
  transfer, trigger sources `hs::UART1_RX/TX`, request enables at
  CR2[6]/[7]) — the embassy repo's `examples/uart_dma.rs` is a complete
  echo example. ⚠ Semantic-level implementation pending board verification
  (the blocking version has no UART DMA recipe).
- **SPI**: `Spi::write_dma` / `Spi::transfer_dma` (dual-channel full
  duplex, trigger sources `hs::SPI1_RX/TX`, request enables at
  CR2.DMARX/DMATX, §22.7) — the embassy repo's `examples/spi_dma.rs` runs
  with zero wiring, and shorting MOSI↔MISO with a jumper wire verifies the
  loopback. ⚠ Semantic-level implementation pending board verification.

Two cross-peripheral scenarios round out the picture: the ADC sequence DMA
(Chapter 11's `read_sequence_dma`, "generalized path pending board check")
and the DAC waveform DMA (Chapter 11's `write_dma`).

## Common Pitfalls

The following four all come from the blocking `dma_demo`'s
verified-on-hardware record (hal repo TESTING.md); the register behavior is
identical in the embassy version:

- **`ICR` is R1W0**: writing 1 does nothing, **writing 0 is what clears**
  (reset value all 1s). The driver already wraps this (`clear_flags`);
  when touching the registers by hand it is extremely easy to step on it
  backwards.
- **`CNT.REPEAT` is cleared to 0 by hardware after every transfer**
  (§8.5.2): to "transfer again" you must re-`configure` the whole channel;
  changing CNT alone does nothing.
- **The software trigger `SOFTSRC` self-clears when the transfer
  completes**: in software-trigger mode, "transfer again" means writing
  SOFTSRC again; with hardware triggers it suffices that the peripheral
  keeps raising requests.
- **The address increment is decided by the width**: the ADC's
  multi-channel result registers RESULT0..7 are 4 bytes apart (Table 25-7),
  which a 16-bit width's 2-byte increment cannot match — either configure a
  32-bit width or move the whole block with `REPEAT` (Chapter 11's
  `read_sequence_dma` uses the latter).
- **Source/destination addresses must not be the DMA/FLASH/RAM controllers'
  own registers** (§8.8.6 note), or STATUS reports an address-out-of-bounds
  class of error.
