# Appendix A The 11-Family Matrix and Porting

embassy-cw32 and cw32-bsp cover all **11 Cortex-M0+ families** of Wuhan Xinyuan
Semiconductor's CW32 lineup. The body of this book uses the CW32L012 as its
teaching thread; this appendix answers two questions:

1. Where do the other families stand today (where does their PAC come from,
   how far does driver coverage go, what are the hardware differences)?
2. What exactly has to change when you move this book's L012 examples onto
   another board?

> Every cell in the tables below was cross-checked against the ecosystem
> repos' README/CHANGELOG/source-file header comments, and sources are cited
> per cell. Anything that could not be confirmed is explicitly marked
> "unverified" — no invented data.

## A.1 The family matrix

**PACs come in two kinds**: `cw32l012-pac-um` for the CW32L012 is
**hand-written** — transcribed item by item from the *CW32L012 User Manual*
V1.4 and cross-corrected field by field against the official SVD (source:
cw32l012-pac README). The PACs of the other 10 families are **generated from
the vendor's SDK header files** by `cw32-pac-gen`, one crate per family
(source: cw32-pacs README). All 11 family PACs are published on crates.io
(2026-11-14): 8 families under their original names, while the L012/F030/L010
trio had to publish with a `-um` suffix (User Manual transcription edition)
because their original names are taken by third parties — the "PAC crate"
column below always states the official crates.io name (naming story in
Appendix B).

| Family | PAC crate | Flash/SRAM | embassy feature name | Reset-default clock | Gating KEY (0x5A5A) | DMA channels | I2C | GPIO ports adopted by the PAC |
|--------|-----------|------------|----------------------|---------------------|---------------------|--------------|-----|-------------------------------|
| CW32L010 | `cw32l010-pac-um` (generated; original name taken) | 64K/4K | `cw32l010` | **4 MHz** | yes | none | yes | A, B |
| CW32L011 | `cw32l011-pac` (generated) | 64K/6K | `cw32l011` | **4 MHz** | yes | none | **no hardware** | A, B, C |
| CW32L012 | `cw32l012-pac-um` (**hand-written**, transcribed from UM V1.4; original name taken) | 64K/8K | `cw32l012` (default) | **4 MHz** | yes | **4** | yes (command-FIFO controller) | A, B, C, F |
| CW32L031 | `cw32l031-pac` (generated) | 64K/8K | `cw32l031` | 8 MHz | no | 4 (L031 group) | yes (classic state machine) | A, B, C, F |
| CW32L052 | `cw32l052-pac` (generated) | 64K/8K | `cw32l052` | 8 MHz | no | 4 (L031 group) | yes (classic state machine) | A, B, C, D, F |
| CW32L083 | `cw32l083-pac` (generated) | 256K/24K | `cw32l083` | 8 MHz | no | **5** | yes (classic state machine) | A, B, C, D, E, F |
| CW32R031 | `cw32r031-pac` (generated) | 64K/8K | `cw32r031` | 8 MHz | no | 4 (L031 group) | yes (classic state machine) | A, B, C, F |
| CW32W031 | `cw32w031-pac` (generated) | 64K/8K | `cw32w031` | 8 MHz | no | 4 (L031 group) | yes (classic state machine) | A, B, C, F |
| CW32F003 | `cw32f003-pac` (generated) | 20K/3K | `cw32f003` | 8 MHz | no | none | yes (interrupt named `I2C`, no digit) | A, B, C |
| CW32F020 | `cw32f020-pac` (generated) | 32K/8K | `cw32f020` | 8 MHz | no | 5 (ch4/5 have no NVIC line, polling only) | yes (classic state machine) | A, B, C, F |
| CW32F030 | `cw32f030-pac-um` (generated; original name taken) | 64K/8K | `cw32f030` | 8 MHz | no (see the KEY footnote below) | **5** | yes (classic state machine, **verified on hardware**) | A, B, C, F |

Sources per column:

- **Flash/SRAM**: the family matrix in the cw32-pacs README (the L012 row
  comes from the cw32l012-pac README). Note that this is the
  **family-maximum configuration** of the PAC's `memory.x` (source: cw32-pacs
  README); a specific part number may be smaller — the CW32F030F6 is 32K, for
  example (source: cw32-probe-rs-compat README variant table).
- **Reset-default clock**: the cw32-bsp README family matrix (L010/L011/L012
  = 4 MHz, the rest = 8 MHz); the embassy-cw32 CHANGELOG records how this
  conclusion came about — "the original text on p14 of the L031 Rev1.8
  datasheet corrected the earlier 4 MHz classification", i.e. each family's
  own datasheet text is authoritative.
- **Gating KEY**: embassy-cw32 README — for the L012 generation
  (`cw32l010`/`cw32l011`/`cw32l012`) SYSCTRL writes are protected by the
  0x5A5A key; the 8 classic-generation families have no key (plain
  read-modify-write) (source: embassy-cw32 README; CHANGELOG "gate-write
  convention" entry).
- **DMA**: the `dma` module row of the embassy-cw32 README and the CHANGELOG's
  `ClassicDma` entry — L012 has 4 channels (IRQ9 DMA12/IRQ10 DMA34); the
  classic generation F030/F020/L083 = 5 channels and the L031 group
  (L031/L052/R031/W031) = 4 channels; **F003/L010/L011 have no DMA hardware**.
- **I2C**: the `i2c` module row of the embassy-cw32 README — 10 of the 11
  families have I2C; **the L011 has no I2C hardware**. The L012 uses a
  command-FIFO style controller; the other 9 use the classic 8051-style
  state-machine controller.
- **GPIO ports**: the per-family `pins.rs` table headers of cw32-bsp (the
  ports each family's PAC adopts; regenerated by cw32-bsp 6a6b773/4c4218b).
  These are the ports **actually modeled by the PAC's `Peripherals`**, and
  they match the physical ports of each family's chips (the generation source
  is the vendor SDK headers).

**Footnote on the F030's KEY**: the F030's AHBEN/APBEN gating registers carry
**no** key protection, but peripheral registers such as CR0/CR1/IER still do —
the PLL recipe in the `rcc` module (transcribed from the vendor library) takes
care of those keyed writes (source: embassy-cw32 README "F030 family notes").

**Unverified cells**: none. All 11 families × 8 columns have a repository
source. Hardware-verification coverage, however, varies a lot: only the L012
and the F030 have on-board records (see A.3). Note one discrepancy between
sources: the "hardware verification" column of the cw32-pacs README says
"register layer, pending on-board test" for **all 10 generated families**
(including the F030), while the embassy-cw32 repo already has on-board F030
records (LCKFB CW32F030C8T7, see below) — the former is stale wording that
was never reclaimed in time; go by embassy-cw32's hardware records (source:
cw32-pacs README hardware-verification column; embassy-cw32 README "F030
family notes").

## A.2 embassy-cw32 module coverage

Before porting, first confirm that the target family has the modules you need
(source: embassy-cw32 README "What you get" table):

| Module | Families covered | Notes |
|--------|------------------|-------|
| `time_driver` / `bind_interrupts!` (the critical-section implementation is provided by cortex-m's `critical-section-single-core`, pulled in with the dependencies) | all 11 families | SysTick one-shot-alarm time driver (1 MHz tick); mis-bound interrupts fail at compile time |
| `gpio` (async) | all 11 families | one shared IRQ per port; on l052/l083 the shared vectors `GPIOC_GPIOD`/`GPIOE_GPIOF` dispatch in pairs |
| `uart` | all 11 families | L012/F030 have UART1/2/3; **l083 has UART1..6** (shared vectors UART1/4, UART2/5, UART3/6 dispatched as pairs); the other families have UART1 only; `BufferedUart` is available on every family |
| `spi` | all 11 families | L012 has SPI1/2/3, F030 SPI1/2, **l083 SPI1/2**, the rest a single instance |
| `i2c` | 10 of 11 (except L011) | L012 is command-FIFO style; the other 9 are classic state-machine style (F030 verified on hardware) |
| `adc` | L012 + F030/F020 (from 0.4.0 on, all 11 families) | L012 has dual ADCs (including `vdda_mv()`/`temperature_mc()`); the classic group is a completely different IP, single-shot/4-result sequence |
| `timer` (PWM / complementary PWM) | L012 + the 7 classic families (l010/l011 still missing) | GTIM edge-aligned PWM with a full scale of 10,000; the classic generation has no shadow registers, takes effect immediately. ATIM complementary PWM: `atim` (L012, ST-style BDTR dead time/brake) + `atim_classic` (f030, PWM2S/DTR double-buffered) — see Chapter 16 |
| `rtc` / `wdt` / `dac` | **L012 only** (from 0.4.0 on, rtc/wdt extend to the classic generation) | the classic generation gets `ClassicRtc`/`ClassicIwdt`/`ClassicWwdgt` (a different IP); `dac` remains L012-only |
| `dma` | L012 (`Dma`) + the 7 classic families (`ClassicDma`) | F003/L010/L011 have no DMA hardware, hence no driver |

> **The 0.4.0 additions in one sentence** (source: the 0.4.0 section of the embassy-cw32 CHANGELOG, published to
> crates.io): classic-family UART/SPI DMA, ADC for all 11
> families, classic GTIM PWM, classic RTC/IWDT/WWDGT, the L012 FLASH driver,
> the L012 OPA, and I2C transactions, plus two-generation ATIM complementary
> PWM (L012 `atim` + f030 `atim_classic`, see Chapter 16) — these are new drivers that this book's
> body does not yet cover; read that CHANGELOG before porting.

Hardware-verification records (which board verified what):

- **CW32L012**: blink_async (2026-10-24; PC13 with an exact 1-second period,
  and a SysTick alarm measured at 1,997,499 ≈ 2,000,000 systicks),
  buffered_echo (including an injected UART1 pend that walks the full
  vector → `bind_interrupts!` → critical-section chain); uart_echo_async was
  flashed and ran with no hardware fault, loopback measurement still waiting
  on a USB-serial adapter (source: embassy-cw32 README "Verification status";
  the on-board debugging record of the time-driver base-value bug is also
  there).
- **CW32F030** (LCKFB Tianmengxing CW32F030C8T7): blink_async_f030 (500 ms
  heartbeat), the UART1 init chain (PA08/PA09 AF2, BRRI=4 @115200), SPI1
  (CR1=0x1E7C), i2c_scan_f030 (112/112 addresses NACK) (source: embassy-cw32
  README "F030 family notes"; CHANGELOG I2C multi-family entry).

## A.3 Porting an L012 example to another family

Take porting the Chapter 7 async blink to the **CW32F030** as the worked
example (the same applies to the other classic families; the L012-generation
families are simpler — see below).

### Step 1: switch the feature name and dependencies

embassy-cw32 picks the family through mutually exclusive features (source:
embassy-cw32 README):

```toml
[dependencies]
embassy-cw32 = { git = "https://github.com/SunWithCloude/Cw32l012_embassy", branch = "main", features = ["rt"] }
# F030: turn off the default feature (the default is cw32l012) and enable cw32f030 instead
# embassy-cw32 = { git = "https://github.com/SunWithCloude/Cw32l012_embassy",
#   branch = "main", default-features = false,
#   features = ["rt", "cw32f030"] }
# (once embassy-cw32 is published on crates.io, switch back to a version dependency)
```

The command-line equivalent (source: embassy-cw32 README "F030 family
notes"):

```bash
cargo build --target thumbv6m-none-eabi --release \
  --no-default-features --features "rt,cw32f030"
```

The PAC switches automatically with the feature, and every one of them comes
from crates.io as a version dependency: L012 uses `cw32l012-pac-um`
(`=0.3.0`), F030/L010 use `cw32f030-pac-um`/`cw32l010-pac-um`, and the other
8 families use their original names (e.g. `cw32f003-pac`, `cw32l031-pac`, all
exact-pinned `=0.1.2`) — no PAC repo needs to be cloned (source:
embassy-cw32 README "Getting started" and its Cargo.toml).

On the blocking side there is no choice: `cw32l012-hal` **covers only the
L012**. The Part 2 examples cannot be ported to another family by "flipping a
feature" — the blocking HAL is, today, L012-only (source: cw32l012-hal
README; cw32-bsp README ecosystem diagram).

### Step 2: verify the reset-default clock

- L012 generation (L010/L011/L012) = 4 MHz; the 8 classic-generation families
  = 8 MHz (source: cw32-bsp README; embassy-cw32 CHANGELOG).
- `Config::default()` already declares the reset-default frequency per family
  (4 MHz HSI 96M/24 and 8 MHz HSI 48M/6); as long as you don't change the
  clock tree you don't need to touch anything (source: embassy-cw32 README
  "Clock convention").
- **A warning counterexample**: it was once assumed that the F030 runs at
  48 MHz after reset; the result was SysTick 6x slow and a 6-second blink
  period. On-board debugging found it, and the DFP's `Dclock=8000000`
  corroborated 8 MHz (source: embassy-cw32 CHANGELOG fix entry). This is
  exactly the family-flavored version of the "clock multiplier" pitfall in
  CONVENTIONS §5.

### Step 3: how to write the gates

- L012 generation: AHBEN/APBEN are **keyed writes** (KEY = 0x5A5A) — that is
  exactly what Chapter 2's `sysctrl::KEY | sysctrl::AHBEN_GPIOA` does
  (source: cw32l012-pac README usage example).
- Classic generation: AHBEN/APBEN have **no key**; use a plain
  read-modify-write (`modify`) (source: the CHANGELOG "gate-write convention"
  entry, cross-checked against the `enable_write_key` metadata of each
  family's clock sidecar in cw32-hal2).
- If you go through the embassy drivers (e.g. `sysctrl_enable_gpioc`) you
  don't have to think about it — the wrappers already differ per family
  (source: embassy-cw32 README examples).

### Step 4: look up the pin alternate functions (the `pins` module)

Pin/AF combinations differ per family; **do not copy the L012 pin numbers**.
Each family's `pins` module in cw32-bsp lists every usable `(port, pin
number, AF number)` combination for UART1/SPI1/I2C1, each entry with its
vendor SDK macro citation (the data source is a located extraction of the
official datasheet AF tables) (source: cw32-bsp README):

- L012: UART1 = PA02(TX)/PA03(RX) **AF1** (verified on hardware; CONVENTIONS
  §5);
- F030: UART1 = PA08(TX)/PA09(RX) **AF2** (verified on hardware; source:
  cw32-bsp README board-support column).

How to look it up (F030 as the example):

```rust
use cw32f030_bsp::pins;
// pins::UART1_TXD / pins::UART1_RXD list every combination
let tx = bsp_pin!(A, p, 8); // PA8 = UART1_TX
let rx = bsp_pin!(A, p, 9); // PA9 = UART1_RX
```

Note that on the F030 the GPIO pins reset to the ANALOG state (`0x9fff`; only
the two SWD lines are digitally enabled). While DIR stays as an input, a UART
START bit can never leave the pin — the embassy drivers such as
`I2c::new_inner` already handle this uniformly; in a bare-register port you
have to clear it yourself (source: embassy-cw32 CHANGELOG I2C multi-family
entry). This is the same species of pitfall as the L012's "PC13 resets to
analog" (CONVENTIONS §5); the difference between families is only the
default value.

### Step 5: blink on single-port / few-port families

On some families the `bsp_pin!` macro currently only expands for port A (see
the sync notes in A.5). When porting blink, start from **PA0** and then move
to the pin where the LED actually sits, per the schematic:

```rust
let mut led = Output::new(Pin::new(p.gpioa, 0), Level::High); // PA0
```

On classic families the LED is often on PC13 (the F030 board is PC13 —
source: cw32-bsp README board-support column); there the PAC has already
adopted port C, and `Pin::new(p.gpioc, 13)` works.

### Step 6: interrupt names and instance counts

- The IRQ names in `bind_interrupts!` must become variants of **the selected
  family PAC's `Interrupt` enum** (the two PAC lines name their enums
  differently — source: embassy-cw32 CHANGELOG `nvic_enable` entry). For
  example, l083's GPIO interrupt is the shared vector `GPIOC_GPIOD`, and
  F003's I2C interrupt name carries no digit — it is just `I2C` (source:
  embassy-cw32 CHANGELOG).
- Check UART/SPI instance counts against the A.2 table: the F030 has
  UART1/2/3, the L031 has only UART1 — `Uart::new_uart2` does not exist on
  single-UART families, and the compiler will tell you right away.

### Step 7: what happens to the L012-exclusive modules

The book walks through the L012 versions of `timer` (PWM), `rtc`, `wdt` and
`dac` chapter by chapter; from 0.4.0 on, embassy-cw32 also brings GTIM PWM,
RTC, and IWDT/WWDGT to the classic generation, and provides ADC drivers for
all 11 families (source: the 0.4.0 section of the embassy-cw32 CHANGELOG) — this book's body does not yet cover those new drivers. When
porting Chapters 10/11/13 to a non-L012 family, either drop down to the
register level (the family's own PAC) or switch to the new drivers directly
(go by that CHANGELOG/README).

### Step 8: flashing and debugging parameters

Chip names and target description files change per family — L010/L011/L012
are built into probe-rs ≥ 0.31.0 (`--chip CW32L012x8` and friends); the other
families need the target yaml from the cw32-probe-rs-compat repo plus
`--chip-description-path`. See Appendix D for details (source:
cw32-probe-rs-compat README).

Two Flash detail pitfalls (source: cw32-probe-rs-compat README "Known
limitations"): the W031's Flash page is **128 B** (512 B on the other
series); the F003's Flash is **20 KB** (`0x5000`, not a power of two) — keep
both in mind when porting flash self-programming code.

## A.4 Porting quick-reference card

| What to change | L012 generation (L010/L011) | Classic generation (the other 8 families) |
|----------------|-----------------------------|--------------------------------------------|
| embassy feature | `cw32l010` / `cw32l011` | the family name (e.g. `cw32f030`) |
| reset clock assumption | 4 MHz | 8 MHz |
| gate write | keyed write, KEY 0x5A5A | keyless read-modify-write |
| UART error bits | CR1 bit 8..11 | FE=bit3/PE=bit4 |
| blink pin | start at PA0 | PC13 (F030 board) / PA0 |
| probe-rs | built in (≥ 0.31.0) | local target yaml |
| DMA | none (L010/L011) | `ClassicDma` (except F003) |
| I2C | L010 has it / L011 does not | present (classic state machine) |

(UART error bits source: embassy-cw32 CHANGELOG — L012 generation "UART error
bits 8..11", classic generation "error bits 3/4". Sources for the other rows
are as cited in the sections above.)

## A.5 Two data-sync notes

When porting, defer to the repos' latest state for the following two items
(wording as of this book's writing):

1. **The `bsp_pin!` macro and the `pins.rs` table headers are out of sync**:
   commits 6a6b773/4c4218b in cw32-bsp regenerated each family's `pins.rs`
   (unlocking gpiob+ once the PAC had re-adopted the ports), but the
   `bsp_pin!` macro in some families' `lib.rs` still expands only the old
   port set (l083's table header says "ports adopted: PA..PF" while the macro
   only expands A, for instance). Before landing a non-A-port pin, read the
   target family's `pins.rs` header first; for ports the macro does not
   recognize, follow its `[PAC]` annotations (source: comparing cw32-bsp
   `crates/<family>-bsp/src/lib.rs` with `src/pins.rs`).
2. **Classic-group DMA channel count**: an early dma entry in the
   embassy-cw32 CHANGELOG says "6 channels", but the later `ClassicDma`
   entry and the README both say F030/F020/L083 = 5 and the L031 group = 4.
   Go with the README/`ClassicDma` entry (5/4); the earlier figure is
   probably stale research wording that was never reclaimed.
