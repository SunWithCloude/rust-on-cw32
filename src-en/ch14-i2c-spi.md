# Chapter 14 I2C and SPI

In this chapter you will learn:

- The model of the L012's command-FIFO-style I2C peripheral, and the
  generational gap from the classic family's (F030) 8051-state-machine I2C
  (§23);
- The constructor parameters of `I2c::new_i2c1` and the minimal bus-scan
  code;
- The SPI master-mode `Spi` driver: software chip select, the `SpiBus`
  async interface (§22).

## I2C: Two Generations of IP, Two Different Worlds

The I2C peripherals of CW32's two family generations are **two entirely
different IPs**, isomorphic nowhere in their register surface:

- **L012 (command-FIFO style, §23)**: the master writes "command + data"
  into `MTDR` — START+address, a TX byte, RXCOUNT (how many bytes to
  receive), and STOP are each one command; progress and errors are read
  from `MISR` (TXE/RXNE/PACKET/NACK/ARBI/FIFO/PINLOW), made async via the
  MIER interrupt + waker. The timing parameters
  (PRESCALE/SETHOLD/CLKHI/CLKLO/DATAVD) are derived by the driver from the
  I2C spec, calibrated against the official examples of manual Table 23-4.
- **Classic family F030 (8051 state-machine style)**: set STA/STO in CR,
  load a byte into DR, and each time SI sets, read a STAT status code
  (0x08/0x10/0x18/…/0x58) — one action per SI, a style inherited directly
  from the 8051's I2C era.

`embassy-cw32`'s `I2c` unifies both behind the same `embedded-hal-async`
interface (`write`/`read`/`transaction`), but **the transaction semantics
differ**: the L012 version executes each Operation independently as
START…STOP (a `Read` following a `Write` does not do a repeated start);
the F030 version runs the whole transaction on a single START and issues a
repeated START on direction switch — stronger, and right for "write
register pointer + read data" devices. When you need cross-direction
transactions, check the corresponding family driver's documentation.

Pin mapping (L012, manual Table 9-2): I2C1's SCL = PA04/PA09, SDA =
PA05/PA10, all AF2; the open-drain electricals are handled by the
peripheral's PINCFG=OD_IO mode, and **the pull-up resistors must be
external** (4.7 kΩ × 2 to VDD).

## The Example: Bus Scan

`examples/embassy/examples/ch14_i2c_scan.rs`: an i2cdetect-style probe — a
START+ADDR+W (empty-data write) is sent to every address in 0x08..0x78;
ACK = hit, NACK = skip. Hit addresses are recorded into a static array
(`HITS`/`MISS`, readable with probe-rs), and the LED gives a visual
verdict: with hits, 900 ms on / 100 ms off; on an empty bus, a slow blink.

```rust
//! Chapter 14 example: I2C1 bus scan (L012).
//!
//! I2C1 master: PA09=SCL / PA10=SDA (AF2, manual Table 9-2; I2C1's other
//! pair PA04/PA05 is also AF2). The bus needs external 4.7kΩ pull-ups to
//! VDD and a common ground with the board. Scans addresses 0x08..0x78
//! periodically (the i2cdetect convention): sends START+ADDR+W (an
//! empty-data probe) to each address; ACK = hit, NACK = skip. Hit
//! addresses are recorded in a static array (the host can read memory via
//! probe-rs to verify).
//!
//! LED (PC13) visual verdict: with hits, 900ms on / 100ms off; without, a
//! 500ms toggle (slow blink). With no slave on the board every address
//! NACKs, so the slow blink is the correct behavior.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch14_i2c_scan -p cw32-book-embassy
//! Status: compiles, not yet run on hardware (L012 command-FIFO I2C driver
//!       semantics pending board check); the scan-example skeleton is
//!       line-for-line same-source as the embassy i2c_scan_f030, which has
//!       been verified on a real CW32F030 board (empty bus 112/112 NACK,
//!       see the embassy-cw32 CHANGELOG). First-flash checklist in the
//!       Chapter 14 text.

#![no_std]
#![no_main]

use core::cell::RefCell;

use critical_section::Mutex;
use cw32_book_embassy as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use embassy_executor::main;
use embassy_time::Timer;
use embedded_hal_async::i2c::I2c as _;

use embassy_cw32::{
    Config, Peripherals, SYSCTRL_KEY,
    gpio::{Level, Output, Pin},
    i2c::{self, I2c},
};

embassy_cw32::bind_interrupts!(struct Irqs {
    I2C1 => i2c::I2c1Irq;
});

/// Scan address range (avoids the reserved 0x00-0x07 segment and the 10-bit address prefix from 0x78)
const SCAN_RANGE: core::ops::Range<u8> = 0x08..0x78;
/// Maximum number of recorded hit addresses (beyond that, only the total is counted)
const MAX_HITS: usize = 16;

/// Scan hit record (stored statically so the host can verify via probe-rs memory reads):
/// `[0]` = number of hits, then the addresses in order, empty slots stay 0.
static HITS: Mutex<RefCell<[u8; 1 + MAX_HITS]>> = Mutex::new(RefCell::new([0; 1 + MAX_HITS]));

/// Miss reason counters (for on-board verification): `[0]`=NACK (no device, normal),
/// `[1]`=bus fault (FIFO/PINLOW), `[2]`=arbitration lost. On an empty bus all should be NACK.
/// `#[used]`: the firmware writes but never reads, keeping the linker from stripping the symbol (the host reads memory).
#[used]
static MISS: Mutex<RefCell<[u8; 3]>> = Mutex::new(RefCell::new([0; 3]));

/// Writes this round's hits and miss reasons into the static records.
fn record_round(hits: &[u8], miss: [u8; 3]) {
    critical_section::with(|cs| {
        let mut slot = HITS.borrow_ref_mut(cs);
        slot[0] = hits.len() as u8;
        for (dst, src) in slot[1..].iter_mut().zip(hits) {
            *dst = *src;
        }
        *MISS.borrow_ref_mut(cs) = miss;
    });
}

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // reset defaults: SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();
    // GPIOA (I2C pins) + GPIOC (LED) in one AHBEN write: KEY whole-word write; two
    // separate writes clobber each other's enable bits (A=4/C=6)
    p.sysctrl.ahben.write(SYSCTRL_KEY | (1 << 4) | (1 << 6));

    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    // Pin muxing/open-drain electricals handled inside the driver (MCR2.PINCFG=OD_IO); pull-up
    // resistors must be external (4.7kΩ × 2 to VDD)
    let mut i2c = I2c::new_i2c1(
        p.i2c1,
        Pin::new(p.gpioa, 9),  // SCL, AF2 (manual Table 9-2)
        Pin::new(p.gpioa, 10), // SDA, AF2
        2,
        Irqs,
        i2c::Config::default(), // default = standard mode 100kHz
    );

    loop {
        // One scan round: send START+ADDR+W to each address; ACK=hit / NACK=skip
        let mut hits = [0u8; MAX_HITS];
        let mut n = 0usize;
        let mut miss = [0u8; 3];
        for addr in SCAN_RANGE {
            match i2c.write(addr, &[]).await {
                Ok(()) => {
                    if n < MAX_HITS {
                        hits[n] = addr;
                    }
                    n += 1;
                }
                Err(i2c::I2cError::Nack) => miss[0] = miss[0].saturating_add(1),
                Err(i2c::I2cError::Arbitration) => miss[2] = miss[2].saturating_add(1),
                Err(_) => miss[1] = miss[1].saturating_add(1), // Fifo / PinLow
            }
        }
        record_round(&hits[..n.min(MAX_HITS)], miss);

        if n > 0 {
            // Devices on the bus: on-dominant blink (a 9:1 on/off ratio reads as two distinct states)
            led.set_low();
            Timer::after_millis(900).await;
            led.set_high();
            Timer::after_millis(100).await;
        } else {
            // Empty bus: slow blink (1s-period toggle)
            led.toggle();
            Timer::after_millis(500).await;
        }
    }
}
```

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch14_i2c_scan -p cw32-book-embassy
```

## SPI: Master Mode and Software Chip Select

The L012's SPI1/2/3 (§22) are conventional full-duplex master/slave
peripherals: CR1 sets MSTR/CPOL/CPHA/bit order, the BR bitfield (7 bits
@CR1[24..31]) sets SCK = PCLK/(BR+1), EN is at CR2[0] — and CR1 must be
written while EN=0, an order the driver's constructor already handles. The
L012's common SPI1 pins: SCK = PA05/PA10, MOSI = PA07/PA12, MISO =
PA06/PA11, all AF3 (Table 9-2; note that PA04/PA05 are also I2C1's and
DAC_OUT's alternate pins — when several peripherals coexist on a board,
allocate pins against Table 9-2 first).

`embassy-cw32::spi::Spi` (constructed with `Spi::new_spi1(p.spi1, sck,
mosi, miso, af, Irqs, Config)`, binding IRQ25 `SPI1`) implements
`embedded-hal-async`'s `SpiBus` trait: `write` / `read` / `transfer`
(full-duplex same-length send + receive) / `transfer_in_place` / `flush`.
**Chip select is software**: the driver does not manage CS — pull a plain
GPIO output low before the transfer and pull it high again after `flush()`
(wait for BUSY to drain) — exactly the timing the blocking `tft_hw_spi`
example uses to drive an ST7789 display (verified on hardware at 24 MHz).
One CS line per slave; with multiple devices sharing the bus, just pull
them low in turn.

The DMA variants (`write_dma`/`transfer_dma`, CR2.DMARX/DMATX, §22.7) were
already mentioned in Chapter 12: the embassy repo's `examples/spi_dma.rs`
runs with zero wiring; the classic family's SpiBus loopback example is the
same repo's `examples/spi_loopback_f030.rs`. Both are semantic-level
implementations pending board verification — read the status note in each
example's header comments before using them.

## On-Hardware Verification

**Status: compiles, not yet run on hardware (L012 path).** This example is
an L012 port of the embassy-cw32 repo's `i2c_scan_f030`: **the example
skeleton is line-for-line same-source** (scan range, hit/miss recording,
and LED verdict logic identical), and **the F030 version has been verified
on real CW32F030 hardware** — an empty bus, 112/112 addresses all NACK
(embassy-cw32 CHANGELOG multi-family entry, 2026-10-24). But the two I2C
generations are different IPs and the L012 driver is a separate
implementation; the L012 side has not been on a board and cannot be counted
as "verified via the same-source example".

First-flash checklist:

- Empty bus: `MISS[0]=112` (all NACK), `MISS[1]=MISS[2]=0`, LED
  slow-blinking; a growing `MISS[1]` means a bus fault (FIFO/PINLOW — check
  the pull-up resistors and common ground first);
- Attach one slave with a known address (e.g. an MPU6050 at 0x68):
  `HITS[0]` should equal the device address and the LED should show the
  on-dominant blink;
- Logic analyzer on SCL: a 100 kHz standard-mode square wave; at the 9th
  clock after the address byte, check that the ACK/NACK level matches the
  analyzer's decode.

## Common Pitfalls

- **Pull-up resistors not attached**: I2C is an open-drain bus; without
  pull-ups SCL/SDA sit at low level and the L012 driver reports
  `I2cError::PinLow` (bus stuck) — measure the static level of both lines
  before blaming the software.
- **Pin-mux collisions**: PA04/PA05 are candidates for both I2C1 and
  DAC_OUT1/2, and PA05/PA06/PA07 are also SPI1 — the L012 is pin-poor; when
  several peripherals coexist, check Table 9-2 item by item, and if
  necessary move I2C to PA09/PA10.
- **L012 transactions have no repeated start**: `transaction(addr,
  &[Write, Read])` on the L012 is two independent transactions (STOP, then
  START); devices that need a repeated START — EEPROM random reads,
  sensor hold registers and the like — must on the L012 either be split
  into two operations (after confirming the device tolerates a STOP without
  resetting its address pointer) or move to the F030 family.
- **CS timing between SPI frames**: pull CS high only after `flush()` has
  drained the shift register; pulling high directly cuts off the last
  byte. Conversely, raising CS mid-transfer drops a frame at the slave.
- **Check CPOL/CPHA against the device manual**: Mode 0 is the most common
  default, but displays/Flash/sensors each have exceptions — when the
  waveform is wrong, check the mode before anything else.
