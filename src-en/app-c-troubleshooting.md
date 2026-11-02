# Appendix C Troubleshooting

Every entry follows the three-part Symptom → Cause → Fix pattern, and the fix
steps can be followed as written. Sources are collected at the end of each
entry; items marked "measured on hardware" have board records (TESTING.md /
embassy-cw32 CHANGELOG / hardware-verification notes).

## C.1 Symptom quick-lookup table

| Symptom | Most likely cause | Fix | Entry |
|------|-----------|------|------|
| probe-rs reports `SwdApWdataError` or other SwdAp* errors, or the target becomes unreachable after one successful flash | sticky error on the MINDP debug port not cleared | **power the target board off completely for a few seconds and re-plug**; if that fails, `probe-rs reset` to reclaim the AP | 1 |
| blink is slower by a fixed integer factor (e.g. 500 ms becomes 3 s / 6 s); all delays are slow in the same proportion | the clock frequency assumed in code ≠ the reset default (L012 generation 4 MHz / classic generation 8 MHz) | recalculate from the reset default; after changing the clock, re-declare with the actual frequency | 2 |
| `nm` cannot find the observation statics; or the readings look like "some other variable" | write-only statics optimized away; addresses drift after recompilation | write observation variables with `write_volatile`; pin symbols with `#[used]`; re-locate with `nm` | 3 |
| link errors about a duplicate/missing critical-section | one firmware image allows only one critical-section implementation | pick **one** of the hal and embassy-cw32 implementations; this book isolates them in separate example crates | 4 |
| `no matching package named ... found` | no manifest at the declared path-dependency path | lay out the repos per §C.2 entry 5 and confirm the directory contains a `Cargo.toml` | 5 |
| LED stays dark | check analog state → gating → polarity, in that order | see the three-step sequence in entry 6 | 6 |
| no waveform on UART/SPI although the register setup looks right | wrong pin AF number (differs between families) | consult the full table in the cw32-bsp `pins` module for that family | 7 |
| VDDA/BGR channels normal, temperature channel stuck at ≈0 counts, `temperature_mc()` errors | a per-die condition: this chip's TS analog block has no output | run `ts_probe` to establish a baseline; swapping the chip settles it; no software fix | 8 |
| debugger reads of peripheral registers return "same-value phantom reads" | direct debugger reads over APB are not trustworthy (phantom reads measured on hardware) | in firmware, `read_volatile`-snapshot into SRAM statics and read those | C.3 |

## C.2 Entries in full

### Entry 1: SWD wedge (`SwdApWdataError` / sticky error)

**Symptom**: `probe-rs` reports `SwdApWdataError` (or another `SwdAp*` / AP
access error) on connect or flash; the stealthier form is "one successful
flash, then the next attach cannot connect".

**Cause**: the whole CW32 series is MINDP (DPv1, no SWD multidrop). Once the
AP enters the error state its sticky bits are not cleared automatically — the
detector in the compat repo was designed specifically around this ("a fresh
attach cycle for every candidate base address"), and it was measured on
hardware that `reinitialize()` clears the MINDP sticky error (source:
cw32-probe-rs-compat README "工作原理" [how it works] / "自动识别链验证"
[auto-detection chain verification]). Firmware can trigger it too: after an
example ends with `wfi`, the first read repeatedly reports
`JtagGetIdcodeError` and even `--connect-under-reset` times out (measured on
hardware; source: cw32l012-hal TESTING.md `ts_probe3` "坑" [pitfalls]).

**Fix (in order)**:

1. **Power the target board off completely for a few seconds and plug it back
   in** — the first move; most cases recover right there (source: CONVENTIONS
   §5, "if SWD wedges during debugging (SwdApWdataError), power-cycle the
   target board first");
2. If that fails: `probe-rs reset --chip CW32L012x8` — measured on hardware,
   one reset reclaims the AP (source: TESTING.md `ts_probe3`);
3. If the board has no exposed RESET line and none of the above works:
   re-verify with a different board to rule out a board-specific problem;
4. **Prevention**: do not end firmware with `wfi`; use
   `loop { core::hint::spin_loop(); }` instead — data is being written into
   SRAM as it runs, so it can be read at any time while spinning (source: the
   recommendation in TESTING.md `ts_probe3`).

### Entry 2: clock multiplier error (blink N times too slow)

**Symptom**: the blink period is a fixed integer multiple of the expected
value; RTT timestamps and all delays are slow in the same proportion.

**Cause**: the code computes time from the wrong reset-default frequency. The
reset default is a hard fact: **L012 generation (L010/L011/L012) = 4 MHz
(HSI 96M/24); the 8 classic-generation families = 8 MHz (HSI 48M/6)**
(source: CONVENTIONS §5; embassy-cw32 README "Clock convention"). Board case:
an F030 with SysTick configured under a 48 MHz assumption ran SysTick 6x slow
with a 6-second blink period; on-board debugging found it, and the DFP's
`Dclock=8000000` corroborated 8 MHz (measured on hardware; source:
embassy-cw32 CHANGELOG fix entry).

**Fix**:

1. When blink misbehaves, first compute "how many times too slow": 6x → the
   classic 48M/8M assumption error;
2. embassy side: if you don't change the clock tree, keep `Config::default()`
   (it already declares 4/8 MHz per family); if you do change it, switch at
   the register level first and then pass the **actual frequency** to `init`
   (source: embassy-cw32 README "Clock convention");
3. blocking side: `Rcc` frequency changes automatically reconfigure the flash
   wait states, and `rcc.clocks()` returns the current frequency (source:
   cw32l012-hal README rcc section);
4. for any measurement that "uses a delay as the time base", first confirm
   the delay accuracy — `delay_us` was once measured +310% slow; after the
   fix, +0.8% (source: TESTING.md `clk_check`).

### Entry 3: debug/observation symbols optimized away

**Symptom**: `nm <elf>` cannot find the observation statics; or the values
read are clearly "shifted" (measured on hardware: `DS_WAKES` once read as
`LSI_HZ=15`; a drifted `TS_RAW` mistaken for `TEMP_MC`, producing a bogus
-271 °C).

**Cause**: with `opt-level` on, a `static mut` that is written but never read
gets deleted wholesale; static variable addresses drift after every
recompile (source: the two TESTING.md entries "静态量被优化掉" [statics
optimized away] and "静态变量地址漂移" [static address drift]).

**Fix**:

1. Always write observation variables with
   `write_volatile(addr_of_mut!(X), v)` — a volatile write has side effects,
   so the compiler will not dare delete it (source: TESTING.md; confirmed on
   hardware);
2. Bare functions/statics that must stay in the symbol table: under edition
   2024 use `#[used]` + `#[unsafe(no_mangle)]` (source: CONVENTIONS §3.4);
3. **Re-locate after every recompile**: `nm <elf> | grep -E " [bBdD] "` —
   never reuse the previous address (source: TESTING.md "通用操作" [general
   operations], item 1).

### Entry 4: critical-section implementation clash (why this book's examples are split into three crates)

**Symptom**: link failure with critical-section duplicate/missing errors
(e.g. `no critical section implementation`, or symbol/link conflicts when
both implementations are enabled at once).

**Cause**: the `critical-section` crate's contract is **one implementation
per firmware image**. The `critical-section-impl` feature of `cw32l012-hal`
and the PRIMASK implementation that `embassy-cw32` enables by default both
provide a `critical-section` implementation; enabling both in the same image
breaks the link (source: the ⚠ note in embassy-cw32 README "Relationship to
cw32l012-hal"; cw32l012-hal README "Only one critical-section implementation
may exist per binary").

**Fix**:

1. Pick one: if you use the hal, do not enable embassy-cw32 (and vice versa);
2. **This book's approach**: the examples are split into three independent
   crates — `examples/pac`, `examples/hal`, `examples/embassy` — so the
   blocking and async tracks never end up in the same firmware (source: the
   `members` of this book's root `Cargo.toml`; the known-pitfalls list in
   CONVENTIONS §5);
3. If you also use `cortex-m`'s `critical-section-single-core`, you must give
   up the hal's implementation (source: cw32l012-hal README Feature flags
   section).

### Entry 5: path dependency resolution failure (`no matching package found`)

**Symptom**: `cargo build` reports `no matching package named ... found` or
`failed to load source for dependency ...` although the crate name is not
misspelled.

**Cause**: cargo requires that a path dependency's declared path **really
contains a manifest (Cargo.toml)**. This book's examples reference the
ecosystem repos at the relative path `../../..`; one repo not cloned, a
directory name mismatch, or a wrong nesting level breaks resolution
immediately.

**Fix**:

1. Lay out the repos per CONVENTIONS §6 — three directories next to this
   book's repo: `cw32l012-pac`, `cw32l012-hal` (the HAL lives in its
   `cw32l012-hal/` subdirectory), and `embassy-cw32`;
2. Check that the path in each examples/*/Cargo.toml points to a **directory
   containing a Cargo.toml** (note the hal is two levels:
   `../../../cw32l012-hal/cw32l012-hal`);
3. For ports to other families, additionally place `cw32-pacs` side by side
   (the PACs at `../cw32-pacs/crates/<family>-pac`; source: embassy-cw32
   README "Getting started").

### Entry 6: LED-not-lit checklist (analog state → gating → polarity)

**Fix (strictly in order; only move on after the previous step passes)**:

1. **Analog state**: after reset the GPIO is in ANALOG — on the L012 PC13 has
   `GPIOx_ANALOG=1` (source: CONVENTIONS §5); the F030 is harsher still: all
   ports reset to `0x9fff` with only the two SWD lines digitally enabled
   (measured on hardware; source: embassy-cw32 CHANGELOG I2C multi-family
   entry). Clear the analog bits / switch to digital function before
   lighting; the embassy drivers' `Output::new` already handles this; in
   bare-register code you must clear it yourself.
2. **Gating**: whether the bit for the corresponding GPIO port is set in
   AHBEN. The L012 generation uses a **keyed write with KEY = 0x5A5A** — a
   write missing the KEY is silently dropped (source: cw32l012-pac README
   usage example; embassy-cw32 CHANGELOG "gate-write convention" entry); the
   8 classic-generation families have no key, plain read-modify-write.
   "Silently dropped" is the common face of this class of pitfall — in flash
   self-programming, missing `AHBEN.FLASH(bit1)` likewise turns erase/write
   into a no-op with no error (measured on hardware; source: TESTING.md
   `flash_store` pitfall 1).
3. **Polarity**: on boards where the LED's other end goes to VCC (including
   the typical PC13 wiring of this book's user board), the pin lights it
   with a **low** level — `Level::Low` / clear ODR; the opposite if it goes
   to GND. If unsure, toggle first and check with a multimeter or scope.
4. If all of the above is right and it still will not light: check the
   schematic and change pins — user board PC13, F030 board PC13; generic
   templates start at PA0 (source: cw32-bsp README board-support column).
5. Analog peripheral pins are the same pitfall in reverse: PB00 must get
   `into_analog()`, otherwise the digital output driver clamps the node to
   0 V and shorts the OPA output (measured on hardware; source: TESTING.md
   `analog_loop` pitfall 2).

### Entry 7: wrong pin AF

**Symptom**: no waveform from UART/SPI/I2C no matter what you tweak; the
register chain checks out "correct" item by item.

**Cause**: AF numbers and usable pin combinations **differ per family** —
the L012's UART1 = PA02(TX)/PA03(RX) **AF1** (measured on hardware; source:
CONVENTIONS §5); the F030's UART1 = PA08/PA09 **AF2** (verified on hardware;
source: cw32-bsp README board-support column). Copying another family's pin
numbers/AF numbers is guaranteed wrong.

**Fix**:

1. Open the family's `pins` module in cw32-bsp: every `(port, pin number,
   AF number)` combination for UART1/SPI1/I2C1 is in the table, each entry
   with its vendor SDK macro citation (source: cw32-bsp README);
2. Alternate-function input signals (e.g. the ATIM brake input) must be
   configured as an **AF input** — measured on hardware: with the PA06 brake
   pin as a plain GPIO input or floating, BRK sampled low → brake latched →
   `BDTR.MOE` cleared by hardware → no waveform on the pin; switching to
   `into_af_input(Af::Af7)` restored it (measured on hardware; source:
   TESTING.md `atim_deadtime` pitfall);
3. When porting L012 examples to another family, always re-look-up
   pins/AFs; see Appendix A.3, Step 4.

### Entry 8: a temperature sensor with "no output" is a per-die variation, not a code bug

**Symptom**: VDDA and the 1.2 V BGR channels read normally, but the TS
channel (internal temperature sensor) is stuck at 0–7 counts (≈0 V);
`temperature_mc()` returns `AdcError::SensorDead`.

**Cause**: **a per-die condition** — this chip's TS analog block
(bias/output stage) has no output. Every software-side avenue has been
exhausted: 5 enable-order variants, settling time, BGR software restart,
ADC2, sampling times S6–S518+Div8, 90 s+ of warm-up, a controlled POR (after
10 s unpowered and re-plugged, BGR recovered to its theoretical 1492 while TS
stayed 0); the third round, `ts_probe3`, swept six new angles and all came up
empty — the most decisive comparison is that **TSEN=0 and TSEN=1 give
identical readings**, meaning the bias current never reaches the sensor; a
three-point fit of the discharge curve gives a consistent τ≈1.03 µs, and the
node shows the signature of "0 V plus a roughly 100–250 kΩ bleed path"
(measured on hardware; source: cw32l012-hal TESTING.md TS entries — the
`ts_probe` decision matrix, the `ts_probe3` six-angle sweep, and the POR
final review).

**Fix**:

1. Run `ts_probe` first (one flash, 30 s covering the five phases
   A/B/C/curve/S6) to establish a baseline and rule out your own code
   (source: TESTING.md `ts_probe` section);
2. If the baseline matches the known per-die anomaly (TS all 0, BGR
   ≈cal×4095/VDDA ±2%) → swap in another CW32L012 running the same firmware
   for the definitive verdict (source: TESTING.md, "同一份固件换一颗即可一锤定音"
   [same firmware plus a different chip settles it]);
3. There is no software fix; **stop tuning**. The library defends against
   it: `temperature_mc*()` returns `AdcError::SensorDead` when raw < 100
   counts, preventing bogus "-271 °C" data (source: cw32l012-hal README
   Known limitations, "dead TS node on the die"; src/adc.rs
   `AdcError::SensorDead`; the embassy-cw32 CHANGELOG 0.4.0 section converges
   to the same error path).

## C.3 Register-level quick reference (a minefield of small pitfalls)

| Symptom | Cause | Fix |
|------|------|------|
| writing 1 to ICR does not clear the flag | the DMA/UART/GTIM ICRs are **R1W0**: writing 1 does nothing, writing **0** clears | a "write the inverted value" such as `icr.write(!(1<<0 \| 1<<1))` (source: TESTING.md `dma_demo` pitfall 1) |
| DMA transfers only once per round | `CNT.REPEAT` is cleared to 0 by hardware after each transfer | call `configure()` again before each round (source: TESTING.md `dma_demo` pitfall 2) |
| Flash erase/write "looks successful" but nothing happened | `AHBEN.FLASH(bit1)` not enabled, or MODE written as a bare `KEY \| MODE` | `Flash::new` opens the gate automatically; MODE must be **OR-ed into the existing CR1** (source: TESTING.md `flash_store`, both pitfalls) |
| LPTIM/LSI stuck, not running | LSI is **off** by default at reset | `Rcc::enable_lsi()` (read TRIM → `CR1.LSIEN` → poll STABLE) (source: TESTING.md `lptim_deepsleep` pitfall 1) |
| OPA configuration reads back as the reset value | the gate is **APBEN2[9]**, not APBEN1 | enable the correct gate bit (source: TESTING.md `analog_loop` pitfall 1) |
| debugger reads of peripheral registers return "same-value phantom reads" | measured on hardware: SAMPLE/SQRCFR/RESULT1 all read 0x89D | in firmware, `read_volatile`-snapshot into SRAM and read that (source: TESTING.md "通用操作" [general operations], item 3) |
| writes to the time keep timing out while the RTC is stopped | the "wait for WAIT" flow only applies to writing DATE/TIME while **running** | write directly while stopped (source: TESTING.md `rtc_demo` pitfall) |
| a serial read returns a pile of errors / hangs | pending receive errors in the ISR not handled | `Uart::take_error()` drains them in overflow > framing > parity > noise priority order (source: cw32l012-hal CHANGELOG 0.9.0) |
