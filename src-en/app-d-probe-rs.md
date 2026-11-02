# Appendix D — probe-rs Quick Reference

Command cards for flashing, memory reads, and RTT logging. Command behavior
varies with the probe-rs version; **for version semantics, the
cw32-probe-rs-compat repo README is the baseline** (each section of this
appendix cites its source).

## D.1 Two ways to specify the chip

**Method 1: `--chip <chip name>` (works out of the box for the L0 series)**

```sh
probe-rs download --chip CW32L012x8 firmware.elf    # probe-rs ≥ v0.31.0
```

Target definitions for CW32L010/L011/L012 have been built in since
**v0.31.0** (2026-01-17) (PR #3682) (source: status table in the
cw32-probe-rs-compat README).

**Method 2: `--chip-description-path <yaml>` (other series / older probe-rs)**

```sh
# local target files for the 9 series, under the compat repo's targets/
probe-rs download --chip CW32F030C8 \
    --chip-description-path targets/CW32F030_Series.yaml firmware.elf
```

`cargo-embed` / `cargo-flash` support `--chip-description-path` as well; the
VS Code probe-rs debugger uses the `chipDescriptionPath` field of
`launch.json` (source: cw32-probe-rs-compat README "快速开始 B" [quick start
B]). The cw32l012-hal repo's `tools/CW32L0_Series.yaml` is the product of the
same flow (`target-gen` from the vendor DFP 1.0.2), for probe-rs < 0.31
scenarios (source: cw32l012-hal README "Debugging in VS Code").

Common variant names (matching the `probe-rs chip list` style, taken from the
official pdsc device table; source: variant table in the
cw32-probe-rs-compat README):

| Series | Variants |
|------|------|
| CW32L012 | `CW32L012x8` (built in) |
| CW32F030 | CW32F030C8 / K8 / F8 (64K), CW32F030F6 (32K) |
| CW32F003 | CW32F003F4 (20K) |
| CW32L031 | CW32L031C8 / K8 / F8 (64K) |
| CW32L083 | CW32L083VC / MC / RC (256K), CW32L083RB (128K) |
| CW32W031 | CW32W031R8 (64K) |

(See the compat repo README for the full 9-series list.)

## D.2 Common commands

The `$PROBE` / `$CFG` below follow the cw32l012-hal TESTING.md convention:
`--probe 0483:3748 --protocol swd --chip CW32L012x8` and
`--chip-description-path <CW32L0_Series.yaml>` (with newer probe-rs, CFG can
be omitted) (source: TESTING.md "通用操作" [general operations]).

```sh
# Flash (download = write to Flash; no erase needed first)
probe-rs download $PROBE $CFG target/thumbv6m-none-eabi/release/examples/blink

# Reset (let the new firmware run)
probe-rs reset $PROBE $CFG

# Full-chip erase
probe-rs erase --chip CW32L012x8 --allow-erase-all
#   (source: cw32l012-vscode README feature table, "full-chip erase" row)

# Read memory: for SRAM snapshots prefer direct reads
probe-rs read b8 0x20000000 128 $PROBE $CFG     # reads 128 bytes, hexdump with address prefixes
#   b16/b32 are analogous, reading in 16/32-bit units — handy for viewing
#   u16/u32 arrays directly; for the count units of each width, see
#   probe-rs read --help (source: TESTING.md "观察静态变量" [observing
#   statics]; the b8=bytes convention was verified on hardware as written)

# Flash + run + RTT logging in one go (see D.4)
probe-rs run $PROBE $CFG target/thumbv6m-none-eabi/release/examples/ch03_rtt
```

**The complete SRAM read flow** (to observe observation variables/statics):

1. `nm <elf> | grep -E " [bBdD] "` to locate symbol addresses — **they drift
   on every recompile; you must look them up again** (source: TESTING.md
   "通用操作" [general operations], item 1);
2. `probe-rs read b8 <symbol address> <byte count>`; the hexdump has address
   prefixes — when parsing, split each line on `':'` and concatenate the
   bytes in order (source: TESTING.md `ts_probe` section);
3. `probe-rs read` is the first choice — it talks to the probe directly, no
   IDE involved; the extension's debug service
   `http://127.0.0.1:3678/api/memory` is only available while an IDE debug
   session is open (source: TESTING.md "通用操作" [general operations],
   item 2);
4. ⚠ Do **not** read peripheral registers (APB) directly with the debugger —
   phantom reads with three registers all showing 0x89D were measured on
   hardware; for register values, have the firmware `read_volatile`-snapshot
   them into SRAM and read that (source: TESTING.md "通用操作" [general
   operations], item 3).

**`cargo run` passthrough**: the cw32l012-hal repo's `.cargo/config.toml`
already wires the two parameter groups above into a `probe-rs run` runner,
so `cargo run --target thumbv6m-none-eabi --release --example blink` flashes
and runs in one step (source: cw32l012-hal README "Flash" section).

## D.3 Chip identification (out of the box on new versions / behavior on older ones)

| probe-rs version | `--chip CW32L012x8` | `probe-rs info` auto-detection |
|--------------|---------------------|--------------------------|
| < 0.31 | not recognized → need `--chip-description-path` | unavailable |
| 0.31 / 0.32 | **works out of the box** (L0 series built in) | fails: `could not automatically be determined` |
| 0.33+ (after the detection layer was upstreamed) | works out of the box | **auto-detects the whole CW32 series out of the box** |

- Why auto-detection fails on 0.32: the whole CW32 series is MINDP (DPv1, no
  SWD multidrop); the standard ARM debug topology carries no vendor-unique
  information, and the ROM table stops at generic ARM components
  (`--verbose` shows "Debug port version DPv1 does not support SWD
  multidrop") (source: cw32l012-pac docs/probe-rs-feedback/
  issue-auto-detection.md; the "背景" [background] section of the
  cw32-probe-rs-compat README).
- The detection layer's solution reads the **factory-area digital signature**
  (part-number string/Flash/RAM/pin count/UID); it was merged upstream with
  PR #4598 (2026-10-31, issue #4550 closed); the F030 target PR #4549 has
  been merged as well. From the next probe-rs release on, `probe-rs info`
  and auto-attach without `--chip` work for CW32 out of the box (source: the
  top-of-file update note in the cw32-probe-rs-compat README; cw32-bsp
  README ecosystem diagram, "both merged upstream").
- On the upstream version number: CONVENTIONS §5 records "upstream 0.33+
  recognizes it out of the box"; the exact version is whatever probe-rs
  ships.

**The standalone identification tool `cw32-detect`** (shipped in the compat
repo; usable without upgrading probe-rs):

```sh
cd cw32-detect && cargo build --release
./target/release/cw32-detect          # human-readable output
cw32-detect --json                    # JSON (for script integration)
cw32-detect --probe 0483:3748         # pick the probe by VID:PID[:serial number]

# identify + flash combined in one step
probe-rs download --chip $(./target/release/cw32-detect -q) \
    --chip-description-path targets/CW32F030_Series.yaml firmware.elf
```

Exit codes: 0 = identified, 1 = no CW32 found, 2 = probe error.
Identification covers all 12 series; for the L0 families it prints the
built-in variant name directly (e.g. `CW32L012x8`) with no target file
needed. The vendor's part-number string carries a package suffix (the
measured output was `CW32L012C8T6`); the tool normalizes on x and matches it
to `CW32L012x8` (match mode NormalizedX) (source: cw32-probe-rs-compat
README "快速开始 C" [quick start C] and "板上验证" [on-board verification]
sections; on a real board, 2026-10-31).

## D.4 RTT logging

```sh
probe-rs run --chip CW32L012x8 <elf>   # flash + reset + attach RTT automatically
```

- `probe-rs run` prints the logs received over RTT to the terminal while the
  firmware runs; the hal repo has wired it up as the `cargo run` runner
  (source: cw32l012-hal README).
- The firmware-side pair: `defmt-rtt` (log encoding over the RTT channel) +
  `panic-probe` (prints on panic and halts; with
  `features = ["print-defmt"]` it prints defmt format). The
  dev-dependencies of this book's `examples/pac` and `examples/embassy` are
  exactly this pair; embassy-cw32's `rtt_hello` example is the end-to-end
  reference (the defmt timestamp is a hand-written `_defmt_timestamp` using
  embassy-time uptime milliseconds, no extra crate) (source: the
  `rtt_hello` entry in the embassy-cw32 CHANGELOG; this book's
  examples/*/Cargo.toml).
- If no logs arrive, check two things first: whether the RTT control block
  survives the compiler (`#[used]` / `no_mangle`; see Appendix C, entry 3),
  and whether probe-rs was given the same `--chip` the firmware was built
  for (the wrong target affects the RAM-layout scan).

## D.5 Common errors table

| Error | One-line fix |
|------|-----------|
| `NoSupportedProbe` | no probe found: try another USB port / install the driver; with multiple probes, pick one via `--probe <VID:PID[:serial number]>` (the TESTING.md convention `--probe 0483:3748`) |
| `JtagGetIdcodeError` | the protocol fell back to JTAG: add `--protocol swd`; in VS Code set `wireProtocol: "Swd"` (bound to happen — measured on hardware; source: cw32l012-hal README "Two things that cost us time") |
| `SwdApWdataError` / other `SwdAp*` (AP sticky error) | power the target board off for a few seconds and re-plug; if that fails, `probe-rs reset` to reclaim the AP — see Appendix C, entry 1 |
| `The connected chip could not automatically be determined` | 0.31/0.32 have no detection layer; this is expected: pass `--chip` explicitly (source: the probe-rs-feedback record in the cw32l012-pac docs) |
| `Unable to load specification for chip` / chip not in the list | supply a local yaml via `--chip-description-path`, or upgrade probe-rs (source: compat README quick start B) |
| `WaitForDapPinsStuck` / DAP handshake stuck | the target is not running normally (unpowered / held in reset / firmware asleep): check power and the RESET line, try `--connect-under-reset`; avoid ending firmware with `wfi` (source: generic probe-rs errors; for the wfi case see TESTING.md `ts_probe3`) |
| read-protected chip reports "not recognized" | the digital-signature area may be unreadable — remove the protection before identifying (source: compat README "已知限制" [known limitations]) |
| connection drops right after connecting | the target firmware reconfigures the debug pins or entered low power: attach immediately after reset, or check the firmware's sleep/pin configuration (source: the MINDP/sticky mechanism in the compat README) |

On the sourcing of error names: `JtagGetIdcodeError` / `SwdApWdataError` /
"could not automatically be determined" have hardware records (marked in the
table); `NoSupportedProbe` / `WaitForDapPinsStuck` are generic probe-rs error
names (not CW32-specific, not present in the local material repos, and their
wording may vary by version) — go by your own probe-rs output.

## D.6 GDB server and VS Code debugging notes

- **Do not use Cortex-Debug**: it goes through GDB, which needs
  `arm-none-eabi-gdb` + a GDB server (OpenOCD / ST-Link GDB server / J-Link),
  and **no GDB server whatsoever ships target definitions for the CW32L0**.
  The probe-rs VS Code extension speaks DAP directly and needs no GDB
  (source: cw32l012-hal README "Debugging in VS Code").
- The VS Code route: the `probe-rs.probe-rs-debugger` extension +
  `wireProtocol: "Swd"` + `flashingConfig.flashingEnabled`; from probe-rs
  0.32 on, put `CW32L012x8` straight into `chip` (the CW32L0 Series shows up
  in `probe-rs chip list`); `chipDescriptionPath` is only needed below 0.32
  (source: cw32l012-hal README).
- The debug backend of this repo's extension is
  `probe-rs dap-server --single-session` (source: cw32l012-vscode README
  feature table).
- **Memory map and PPB access**: there is a claim that the target yaml's
  memory map may make the debugger refuse access to the PPB region (from
  0xE000_0000: SysTick/NVIC/SCB) — **this could not be found in the current
  cw32-probe-rs-compat README; unverified**. If you hit "cannot read the core
  peripherals", defer to the compat repo's latest records and the probe-rs
  source of your version. The nearby issue with a hardware record is the
  phantom read of APB peripheral registers (fix in D.2, item 4; source:
  TESTING.md).

## D.7 Version behavior differences

The command examples in this appendix span probe-rs 0.31/0.32 and the
before/after of upstreaming, and behavior differs: the out-of-the-box
`--chip` coverage, auto-detection, the count semantics of `read` at each
width, and more can all change between versions. **When a troubleshooting
command does not work, check the version before the command** — the
cw32-probe-rs-compat repo README (and its "已知限制" [known limitations]
section) is the baseline; the compat repo itself depends on the probe-rs
0.32.x public API (source: compat README "已知限制" [known limitations]).
