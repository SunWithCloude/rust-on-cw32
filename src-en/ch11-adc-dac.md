# Chapter 11 ADC and DAC

In this chapter you will learn:

- The channel model and scan sequences of the L012's dual ADC instances
  (ADC1/ADC2) (§25);
- How to use the internal channels: back-computing VDDA from the 1.2 V
  reference, the temperature sensor, and their factory calibrations;
- The difference between the three read paths `read` / `read_sequence` /
  `read_sequence_dma`;
- The DAC's dual-channel direct writes and the request-driven model of DMA
  waveform output (`write_dma`) (§26).

## ADC: Two Instances and Channel Mapping

The L012 has two 12-bit SAR ADCs (§25). **ADC1 covers all channels; ADC2
has only the external channels**, and their external pin mappings even
differ (Table 25-4):

- ADC1: IN0..7 = PA0..PA7, IN8 = PB0, IN9 = PB1, IN10 = PB10, IN11 = PB2;
  IN12/IN13 are the internal interconnects from the DAC outputs; IN14 =
  temperature sensor, IN15 = 1.2 V reference (these two internal channels
  exist only on ADC1);
- ADC2: a separate mapping starting at IN0 = PA05, with no internal
  channels.

External pins must first be switched to analog mode (`Pin::into_analog()`,
setting `GPIOx_ANALOG.PINy=1`, the analog-channel requirement of §25).

The ADC clock ADCCLK = PCLK/2^N (`AdcClk::Div1..Div8`, the CLK bitfield of
the §25 CR register), and each channel independently selects a sample-time
step (`SampleCycles::S6..S518`, in ADCCLK units, Table 25-2) — long sample
times for high-impedance sources is the universal SAR ADC rule.

## Three Read Paths

The `Adc` driver (constructed with `Adc::new_adc1(p.adc1, AdcClk::Div2,
Irqs)`, binding IRQ12 `ADC1`) provides three async read paths:

- **`read(ch, sample)`** — convert one channel once, wake on the EOC
  (conversion complete) interrupt, return 0..4095;
- **`read_sequence(&chs, &samples, &mut out)`** — sequence scan: one start
  converts 2..=8 channels in order, and after EOS (the whole sequence
  completes) `out` is filled in channel order;
- **`read_sequence_dma(&mut dma, &chs, &samples, &mut out)`** — the DMA
  version of the above: when the sequence completes (EOS) the ADC raises
  one DMA request, and the DMA channel moves the whole batch of results
  into the buffer in one go, with the CPU not fetching channel by channel.

All three paths handle the preconditions of the internal channels
automatically: BGREN (the 1.2 V reference enable, already on at reset) and
TSEN (the temperature sensor), and asynchronously wait out their settling
times.

## Internal Channels and Factory Calibration

**VDDA measurement**: read IN15 (the 1.2 V reference) to get raw, combine
it with the 16-bit factory calibration value cal at Flash information-block
address 0x0010_07D2, and compute `VDDA = cal × 4095 / raw` (manual §25.8
step 13). `adc.vdda_mv().await` returns millivolts directly.

**Temperature measurement**: read IN14 (the temperature sensor) and apply
`T = T0×0.5 + 0.095765×(Vref×AdcValue − 4.095×Trim)` (§25.9); T0/Trim
likewise come from factory calibration words. `temperature_mc()` returns
millicelsius and measures VDDA first automatically.

⚠ **The temperature sensor having no output on some silicon is a known
unit-to-unit variation**: through three rounds of controlled experiments
(enable order, sample-time steps, before/after POR comparison) the author
of this book confirmed that the TS analog block's output stage on that chip
produces nothing (0~7 counts), while BGR/external channels on the same path
are all normal — a hardware unit state, not a driver problem. For this
reason the driver adds a fail-safe: when the raw value is <100 counts
(normal room temperature ≈750~1100), `temperature_mc*()` returns
`AdcError::SensorDead` instead of computing fake data like "-271℃". Before
reading the temperature on your own board, first check whether `read()`'s
raw value is in the normal range.

## DAC: Direct Writes and Request-Driven DMA

The L012's DAC is dual-channel 12-bit (§26): CH1's pin output is PA4
(DAC_OUT1) and CH2's is PA5 (DAC_OUT2) — note that these two pins are also
SPI1's alternate pins, and the official development boards assign them to
the display; **CH2 is wired directly to ADC1_IN12 on-chip**, without going
through a pin, so a zero-wiring self-test of "write DAC → read back with
ADC" is always available (the blocking version measured a maximum error of
3 LSB, see the hal repo's TESTING.md).

Direct-write mode (reset default, TENx=0): `set_ch1/set_ch2/set_both` write
DHR and the value lands in DOR within 1 dac_pclk, changing the output
immediately — this is the path verified on hardware.

DMA waveform output (§26.7) is a **request-driven** model: when
`CR0.DMAENx=1` and the DMA channel's hardware trigger source = the
corresponding DAC channel (Table 8-8: `hs::DAC1`/`hs::DAC2`), each DAC DOR
load automatically requests the DMA to move the next sample into DHR. The
per-sample timing is therefore "trigger → play the current sample → raise
the request → DMA preloads the next sample", and **N samples need N+1
triggers to be played out completely** — `write_dma` finishes by playing
the last sample into DOR through the direct-write path, presenting the
caller with "the whole buffer has been played". Use a timer's TRGO as the
beat source (Table 26-1 to pick TSEL); this example uses GTIM1 configured
for a 1 kHz update trigger.

## Example 1: ADC External Channel + VDDA

`examples/embassy/examples/ch11_adc.rs`: runs with zero wiring. With PA0
(IN0) floating the reading is indeterminate; attach a potentiometer/voltage
divider and it follows the input; VDDA goes through the internal reference
+ factory calibration, expected 3200~3300 mV.

```rust
//! Chapter 11 example: ADC1 async single conversion (external channel
//! PA0/IN0 + internal reference for VDDA).
//!
//! Runs with zero wiring: PA0 floating reads indeterminately (proves the
//! path itself); VDDA goes through the IN15 internal 1.2V reference +
//! factory calibration, expected ~3200-3300 mV. PC13 toggles every round.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch11_adc -p cw32-book-embassy
//! Status: compiles, not yet run on hardware (same state as the embassy
//!       version's adc_async); the VDDA/internal-channel recipe is
//!       same-source as the blocking adc_internal and verified on hardware
//!       (VDDA=3282mV, see the hal repo's TESTING.md). First-flash
//!       checklist in the Chapter 11 text.

#![no_std]
#![no_main]

use embassy_executor::main;
use embassy_time::Timer;

use cw32_book_embassy as _; // inherit the panic handler from lib.rs (same mechanism as panic-probe)
use embassy_cw32::{
    Config, Peripherals, SYSCTRL_KEY,
    adc::{Adc, AdcChannel, AdcClk, SampleCycles},
    gpio::{Level, Output, Pin},
};

embassy_cw32::bind_interrupts!(struct Irqs {
    ADC1 => embassy_cw32::adc::Adc1Irq;
});

/// Latest round's results: [0]=VDDA (mV), [1]=PA0 raw code (0..4095).
/// Single-task exclusive writes + probe-rs read-only, so static mut is acceptable.
#[used]
#[unsafe(no_mangle)]
pub static mut RESULTS: [u32; 2] = [0; 2];
/// Error code: 0=none 1=Timeout 2=BadCalibration 3=UnsupportedChannel
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;
/// Completed rounds (heartbeat counter)
#[used]
#[unsafe(no_mangle)]
pub static mut SAMPLES: u32 = 0;

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // reset defaults: SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // GPIOA (PA0 analog pin) + GPIOC (PC13 LED) in one AHBEN write: KEY whole-word
    // write; two separate writes clobber each other's enable bits (A=4/C=6)
    p.sysctrl.ahben.write(SYSCTRL_KEY | (1 << 4) | (1 << 6));
    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    let mut adc = Adc::new_adc1(p.adc1, AdcClk::Div2, Irqs);
    // PA0 → analog mode (IN0; analog peripherals require GPIOx_ANALOG.PINy=1)
    let _pa0 = Pin::new(p.gpioa, 0).into_analog();

    loop {
        // VDDA: internal 1.2V reference channel + factory calibration (BGREN settling handled by the driver)
        match adc.vdda_mv().await {
            Ok(mv) => unsafe { RESULTS[0] = mv },
            Err(e) => unsafe {
                ERR = match e {
                    embassy_cw32::adc::AdcError::Timeout => 1,
                    embassy_cw32::adc::AdcError::BadCalibration => 2,
                    _ => 3,
                }
            },
        }
        // PA0 external channel (floating reads indeterminately; attach a potentiometer/voltage divider and it follows the input)
        if let Ok(raw) = adc.read(AdcChannel::In(0), SampleCycles::S24).await {
            unsafe { RESULTS[1] = raw as u32 };
        }
        unsafe {
            SAMPLES += 1;
            led.toggle();
        }
        Timer::after_millis(500).await;
    }
}
```

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch11_adc -p cw32-book-embassy
```

## Example 2: DAC DMA Triangle Wave

`examples/embassy/examples/ch11_dac_wave.rs`: GTIM1 configured for a 1 kHz
update + TRGO (CR2.MMS=010), DAC1's trigger source set to
`DacTrig::Gtim1Trgo`, and `write_dma` playing a 32-point triangle wave (one
round per 32 ms). Pin output is not enabled by default (PA4 conflicts with
the SPI1 pins); to see the waveform, add one line `dac.enable_pin1()` and
put the probe on PA4. The zero-wiring pass criterion is `ERR=0` with
`ROUNDS` increasing and `LAST_DOR` constant 0 (the triangle's end point =
the last-sample replay taking effect).

```rust
//! Chapter 11 example: DAC1 DMA triangle-wave output (GTIM1 TRGO beat,
//! runs with zero wiring).
//!
//! Trigger chain: GTIM1 configured for a 1kHz update + TRGO
//! (CR2.MMS=update) → DAC1 trigger source TSEL=GTIM1_Trgo → each trigger
//! plays one sample and requests the DMA to preload the next (`write_dma`,
//! DMA channel 2). A 32-point triangle wave = one round per 32ms; PC13
//! toggles every round.
//!
//! Build: cargo build --release --target thumbv6m-none-eabi --example ch11_dac_wave -p cw32-book-embassy
//! Status: compiles, not yet run on hardware (same state as the embassy
//!       version's dac_wave; the DAC's DMA request-driven semantics come
//!       from manual §26.7, but the TRGO beat × request line × last-sample
//!       replay timing has not been checked on a board). First-flash
//!       checklist in the Chapter 11 text.

#![no_std]
#![no_main]

use embassy_executor::main;
use embassy_time::{Duration, Timer, with_timeout};

use cw32_book_embassy::board;
use embassy_cw32::{
    Config, Peripherals,
    dac::{Dac, DacChannel, DacError, DacTrig},
    pac,
};

embassy_cw32::bind_interrupts!(struct Irqs {
    DMA12 => embassy_cw32::dma::Dma12Irq;
    DMA34 => embassy_cw32::dma::Dma34Irq;
});

/// 32-point triangle wave (0→4095→0, 16 points up, mirrored down; one round per 32ms at the 1kHz beat)
static TRIANGLE: [u16; 32] = {
    let mut a = [0u16; 32];
    let mut i = 0;
    while i < 32 {
        let k = if i < 16 { i } else { 31 - i };
        a[i] = (k as u32 * 273) as u16; // peak 15×273=4095
        i += 1;
    }
    a
};

/// Completed full-playback rounds (heartbeat: one DMA TC wake per round)
#[used]
#[unsafe(no_mangle)]
pub static mut ROUNDS: u32 = 0;
/// Error code: 0=none 1=timeout (beat source stopped?) 2=DMA underrun 3=DMA channel error
/// 4=value out of range 5=invalid buffer length
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;
/// DOR1 after the last completed playback (expected constant 0 = triangle end point, last-sample replay in effect)
#[used]
#[unsafe(no_mangle)]
pub static mut LAST_DOR: u32 = 0;

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // reset defaults: SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // GPIOC clock + PC13 LED heartbeat: gating + construction inside board::led()
    let mut led = board::led();

    // ---- GTIM1 → TRGO beat (1kHz update; Table 26-1 TSEL=0100 = GTIM1_Trgo) ----
    // APBEN1.GTIM1 (bit6, KEY read-modify-write — preserves other peripherals' enable bits)
    p.sysctrl.apben1.modify(|v| {
        pac::sysctrl::KEY | (v & 0xFFFF) | pac::sysctrl::APBEN1_GTIM1
    });
    let gtim1 = p.gtim1;
    gtim1.cr1.write(0); // stop counting so it can be configured
    // PSC=3, ARR=999: PCLK 4MHz /(3+1)/(999+1) = 1kHz update
    gtim1.psc.write(3);
    gtim1.arr.write(999);
    // CR2.MMS[6:4]=010 (update → TRGO; the PAC has not absorbed this bitfield, so write the raw value)
    gtim1.cr2.write(2 << 4);
    gtim1.egr.write(pac::gtim::EGR_UG); // reload the shadow registers
    gtim1.cr1.write(pac::gtim::CR1_EN); // run, start providing the beat

    // ---- DAC + DMA waveform chain (pin output not enabled by default: PA4 conflicts with
    //      the SPI1 pins; add one line dac.enable_pin1() when you need a scope, probe on PA4) ----
    let mut dac = Dac::new(p.dac);
    let mut dma = embassy_cw32::dma::Dma::new(p.dma, Irqs);
    dac.set_trigger(DacChannel::Ch1, DacTrig::Gtim1Trgo);

    loop {
        // 32 samples @1kHz = 32ms/round; a 2s timeout as a safety net (no hang if the beat source stops)
        let r = with_timeout(
            Duration::from_millis(2000),
            dac.write_dma(&mut dma, DacChannel::Ch1, &TRIANGLE),
        )
        .await;
        match r {
            Ok(Ok(())) => unsafe {
                ERR = 0;
                LAST_DOR = dac.output1() as u32;
            },
            Ok(Err(DacError::DmaUnderrun)) => unsafe { ERR = 2 },
            Ok(Err(DacError::Dma(_))) => unsafe { ERR = 3 },
            Ok(Err(DacError::ValueOutOfRange)) => unsafe { ERR = 4 },
            Ok(Err(DacError::BadLength)) => unsafe { ERR = 5 },
            Err(_) => {
                // timeout: the trigger chain never started / requests not arriving — stop the channel to clean up, then retry
                dac.stop_dma(&mut dma, DacChannel::Ch1);
                unsafe { ERR = 1 }
            }
        }
        unsafe {
            ROUNDS += 1;
            led.toggle();
        }
        Timer::after_millis(468).await;
    }
}
```

Build command:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch11_dac_wave -p cw32-book-embassy
```

## On-Hardware Verification

- **ch11_adc: compiles, not yet run on hardware.** Same-source as the
  `adc_async` example in the embassy-cw32 repo; the VDDA/internal-channel
  register recipe is same-source as the blocking `adc_internal`/
  `adc_sequence` and verified on hardware (hal repo TESTING.md: VDDA=3282
  mV, BGR raw code 1488 ≈ the theoretical 1489). First-flash checks:
  `RESULTS[0]` in 3200~3300 mV, `RESULTS[1]` following PA0's level,
  `ERR=0`; the embassy async path itself (EOC/EOS interrupt wake-ups) still
  awaits board verification.
- **ch11_dac_wave: compiles, not yet run on hardware.** Same-source as the
  `dac_wave` example in the embassy-cw32 repo, and that example's source
  comments state explicitly "the DAC DMA mode is an inferred implementation,
  pending board verification": the §26.7 request-driven semantics have
  manual text behind them, but the timing of `hs::DAC1` request line × GTIM
  TRGO beat × last-sample replay has never been checked on a board (the
  blocking version has no DAC DMA recipe). First-flash checks: `ERR=0`,
  `ROUNDS` increasing (the trigger chain is alive), `LAST_DOR` constant 0
  (the replay is in effect); a scope on PA4 should show a 31 Hz triangle
  wave (1 kHz ÷ 32 points). `ERR=2` (DMA underrun) means the trigger
  outruns the DMA; `ERR=1` (timeout) means the request chain is not getting
  through — the conservative fallback is to replace `write_dma` with direct
  DHR writes from a timer interrupt.

## Common Pitfalls

- **ADC2 has no internal channels**: calling `vdda_mv()`/`temperature_mc()`
  on an instance built with `new_adc2` returns
  `AdcError::UnsupportedChannel`.
- **Forgot `into_analog()` on the external pin**: the digital output driver
  clamps the pin at 0 V and the reading stays 0 — a lesson measured the
  hard way in the hal repo's analog_loop example.
- **A sequence takes 2..=8 channels**: single channels use `read`; a
  sequence needs at least two; an undersized `out` buffer is rejected
  wholesale.
- **`read_sequence_dma`'s recipe is a generalization of the single-shot
  path**: what the blocking version verified on hardware is single-shot
  conversion DMA (DMAEOC + `ADC1_SINGLE`); moving the whole sequence block
  (DMAEOS + `ADC1_SEQ`, one request moves REPEAT elements) is generalized
  from the manual's data-block model — on first flash, check the buffer
  contents against a per-channel read-back.
- **DAC channel enable conflicts with the SPI pins**: `enable_pin1/
  enable_pin2` hand PA4/PA5 to the DAC; if the same board also needs SPI1
  you must pick one; the internal interconnect (the ADC loopback) is
  unaffected by pins.
- **`write_dma` has no built-in timeout**: if the beat source stops, it
  suspends forever; the example wraps it in `embassy_time::with_timeout` as
  a safety net.
