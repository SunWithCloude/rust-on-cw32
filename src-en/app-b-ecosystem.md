# Appendix B The Ecosystem at a Glance

Every crate and tool this book uses comes from the open-source repositories
under [SunWithCloude](https://github.com/SunWithCloude). This appendix gives
the panorama: who occupies each layer of the three-layer architecture, what
the publication status is, and how it all relates to the vendor's C SDK.
Version information follows each repo's README/CHANGELOG as of the time of
this book's writing.

## B.1 The three-layer architecture

```
                    Your firmware (this book's three examples/ crates)
      ┌───────────────────────────────────┬───────────────────────────────────┐
      │ Blocking track (Chapters 2-6)     │ Async track (Chapters 7-14)       │
      │ cw32l012-hal                      │ embassy-cw32                      │
      │ blocking drivers + built-in       │ embassy-executor + SysTick time   │
      │ runtime, zero external deps       │ driver; 11 mutually exclusive     │
      │ (default); covers L012 only       │ family features                   │
      └────────┬──────────────────────────┴──────────┬────────────────────────┘
               │ built-in pac module (same source)   │
      ┌────────┴──────────────────────────┐  ┌───────┴───────────────────────────┐
      │ cw32l012-pac                      │  │ cw32-pacs (10 crates)             │
      │ hand-written, transcribed from    │  │ generated from vendor SDK headers │
      │ UM V1.4, cross-corrected item by  │  │ by cw32-pac-gen; managed as a     │
      │ item against the official SVD     │  │ single workspace                  │
      └───────────────────────────────────┘  └───────────────────────────────────┘
        PAC layer: no_std, zero runtime dependencies, svd2rust-style API
        (Peripherals::take()/steal(), read()/write()/modify())

      Board layer (optional): cw32-bsp — full pins tables for 11 families + bsp_pin! + blink templates
      Toolchain (independent of the firmware layers):
        probe-rs ←—— cw32-probe-rs-compat (target YAML + cw32-detect identification)
        cw32-tools (VS Code extension: build / flash / visual debugging / Agent API)
```

(Structure: merged and redrawn from "在 CW32 Rust 生态中的位置" [position in
the CW32 Rust ecosystem] in the cw32-pacs README and "生态位置" [ecosystem
position] in the cw32-bsp README.)

Two key points:

- **The PAC layer is dual-track**: the L012 has a repo of its own (published
  before cw32-pacs; it already backs the hal and on-hardware debugging, with
  an independent release cadence and register-patch flow), while the other 10
  families share the generator `cw32-pac-gen`, whose output style matches the
  L012 PAC (source: the note under the family matrix in the cw32-pacs
  README).
- **Two HAL tracks coexist; pick one as needed** (source: embassy-cw32 README
  "Relationship to cw32l012-hal"):

| | cw32l012-hal | embassy-cw32 |
|---|---|---|
| Model | blocking + built-in runtime | async / embassy |
| Dependencies | none (zero deps by default) | the embassy ecosystem |
| Vector table | built in (`UART1_irq` style) | cortex-m-rt + PAC `rt` (`UART1` style) |
| Family coverage | L012 only | 11 families |

⚠ The two tracks' `critical-section` implementations **cannot coexist in the
same firmware image** (linking fails if the hal's `critical-section-impl` and
embassy-cw32's default implementation are enabled at the same time) — which is
why this book switches to separate example crates from Chapter 7 on; see
Appendix C, entry 4 (source: embassy-cw32 README; CONVENTIONS §5).

## B.2 Role and version status of each repo

| Repository | Role | crates.io | How to pull it in | Version status |
|------|------|-----------|----------|----------|
| `cw32l012-hal` | blocking HAL + runtime for the L012 | **published** | `cw32l012-hal = "0.8"` | local Cargo.toml **0.9.1**; the README badge points to crates.io/docs.rs; the v0.9 ecosystem features (critical-section/embedded-io/defmt) are verified on hardware |
| `cw32l012-pac` | L012 PAC (hand-written) | **unpublished** — the name is taken | path dependency | local 0.2.1; its register data is byte-for-byte the same source as the hal v0.9.1 pac module |
| `cw32-pacs` | PACs for the other 10 families | **release pending** | git dependency | crates.io release "to be scheduled once trademark/naming cooperation with the vendor is confirmed" |
| `embassy-cw32` | async HAL for the 11 families | **unpublished** | git/path dependency | Cargo.toml 0.3.0; the CHANGELOG has settled 0.1.0/0.2.0 (both 2026-10-24); the "0.4.0 increments" section is unreleased |
| `cw32-bsp` | board-support templates for the 11 families | unpublished | git dependency (`cw32f030-bsp = { git = … }`) | no version number; the README gives git usage |
| `cw32-probe-rs-compat` | probe-rs target YAML + identification tool | tool repo (no library published) | clone and use | `cw32-detect` builds on the probe-rs 0.32.x public API |

Sources: cw32l012-hal README (crates.io badge, usage section, Roadmap v0.9
entry) and Cargo.toml; cw32l012-pac README ("Publishing note", Verification
section) and Cargo.toml; cw32-pacs README ("发布状态" [publication status]);
embassy-cw32 Cargo.toml and CHANGELOG; cw32-bsp README ("用法" [usage]);
cw32-probe-rs-compat README ("已知限制" [known limitations]).

Two naming traps that are easy to trip over:

1. **`cw32l012_pac` 0.1.1 on crates.io is not the repo this book uses.** That
   one is generated from the vendor SVD by svd2rust v0.37.1 (published
   2026-06-25). Because that name is taken, the `cw32l012-pac` used in this
   book will have to publish under a different name (e.g. `cw32l012-pac-um`)
   — path dependencies are unaffected. The SVD version has three known
   defects (`GPIOx_PDR @ 0x0C` missing, `IRQ31 = FAULT` missing,
   `SYSCTRL.PLL @ 0x28` leftover); the hand-written PAC corrects them per UM
   V1.4 (source: cw32l012-pac README "Why not the cw32l012_pac 0.1.1";
   cw32l012-hal README "Register-level confidence").
2. **The official GitHub repo of `embassy-cw32` was originally named
   `Cw32l012_embassy`** — the README links in cw32-bsp/cw32-pacs still point
   there, and the repo's content is the 11-family async HAL (source: the
   first-line link of the cw32-bsp README; cw32-pacs README ecosystem
   diagram).

## B.3 Toolchain repos

Beyond the three firmware layers there are three tooling-oriented repos
(source: cw32-pacs README; cw32-bsp README ecosystem diagram):

- **cw32-bsp**: strictly speaking "board support" rather than a pure tool —
  one crate set covering 11 families, providing the full pin
  alternate-function tables (the `pins` module: every UART1/SPI1/I2C1
  combination + AF number, each entry with its vendor SDK macro citation),
  the `bsp_pin!` macro, and injectable blink/uart_echo templates (source:
  cw32-bsp README).
- **cw32-probe-rs-compat**: fills in the two layers probe-rs is missing for
  CW32 — the target layer (YAML for 9 series, converted from Flash
  algorithms extracted from the official DFP packs) and the auto-detection
  layer (`cw32-detect`, which reads the part-number string from the factory
  area's "digital signature"). The detection layer has been upstreamed:
  PR #4598 merged (2026-10-31), issue #4550 closed; from the next probe-rs
  release on, `probe-rs info`/auto-attach works for CW32 out of the box; the
  F030 target PR #4549 has been merged upstream as well (source: the
  top-of-file update note in the cw32-probe-rs-compat README — that table's
  row still says "open", stale wording; the cw32-bsp README ecosystem diagram
  states plainly "#4549 F030 flashing + #4598 all-series auto-detection, both
  merged upstream"). See Appendix D for details.
- **cw32-tools** (VS Code extension; package name in the repo
  `cw32l012-tools`, "CW32L012 Tools"): positioned as "build / flash / visual
  debugging / Agent API", mirroring Keil's compile → flash → debug workflow
  (build; probe-rs flash/reset/full-chip erase; native VS Code breakpoint
  debugging via `probe-rs dap-server --single-session`). It ships an HTTP
  JSON Agent API listening only on 127.0.0.1:3678 (`/api/status`,
  `/api/build`, `/api/flash`, `/api/memory`, etc.) that AI agents and scripts
  can curl directly. Version status: README/`package.json` 0.5.0 (0.6.0 in
  development: serial monitor folded into the panel, verify-after-flash,
  etc.) (source: the local checkout's README and docs/AGENT_API.md; the
  recorded behavior that its `api/memory` "is only available while an IDE
  debug session is open" is in cw32l012-hal TESTING.md "通用操作" [general
  operations]). The body of this book does not depend on the extension.

## B.4 Relationship to the vendor C SDK / DFP

The Rust ecosystem did not reinvent the data; it converts layer by layer,
starting from the vendor's assets. The mapping:

| Vendor asset | Role in the Rust ecosystem | Source |
|----------|----------------------|------|
| *User Manual* / *Datasheet* | the **sole source of the hand-written transcription** of the L012 PAC (UM V1.4); data source for the BSP pin AF tables (located extraction via cw32-hal2 `cw32-data/af`); family reset clocks follow each family's own datasheet text | cw32l012-pac README; cw32-bsp README; embassy-cw32 CHANGELOG |
| C standard peripheral library (SDK headers) | generation input for `cw32-pac-gen` (the 10 family PACs); reference source for classic-group driver recipes (the F030 `rcc` PLL recipe transcribed from the vendor library, classic-group ADC per the vendor library's V2.2 recipe, the I2C BRR formula, DMA trigger-source tables in each family's manual §8.8.4) | first paragraph of the cw32-pacs README; multiple embassy-cw32 CHANGELOG entries |
| SVD | generation source of the upstream `cw32l012_pac` 0.1.1; **not used by this book's ecosystem** (three defects, see B.2) | cw32l012-pac README |
| DFP pack (e.g. `WHXY.CW32L012_DFP` 1.0.2) | generation source of the probe-rs target YAMLs — Flash algorithms (FLM) converted to YAML via `target-gen`/equivalent scripts; the compat repo's script chain `extract_all.py → cw32_gen.py` reproduces the same flow from the official firmware-library zip | cw32l012-hal README "Flash" section; cw32-probe-rs-compat README "目录结构/复现流程" [directory structure / reproduction flow] |

The conversion quality has cross-check records on file (source:
cw32-probe-rs-compat README "验证情况" [verification status]):

- The `CW32F030_Series.yaml` generated in the compat repo is **byte-for-byte
  identical** to the algorithm blob of PR #4549 (hardware verified) (632 B);
- The L0-series algorithm produced by the same-source flow is **byte-for-byte
  identical** to the upstream-merged `CW32L0_Series.yaml` (412 B);
- On the PAC side: the hand-written L012 PAC has host-side regression tests
  (register-block size/offset, base addresses, and the 32 IRQ numbers all
  agree three ways) plus an on-hardware `rt` smoke test (source:
  cw32l012-pac README "Verification").

One more piece of related work outside this ecosystem worth knowing about:
fan2nd/cw32-hal2 (October 2026, predating embassy-cw32) takes the
chiptool/metapac all-families route with a very broad driver surface;
embassy-cw32 chose the "one PAC per family" route and absorbed the
`BufferedUart` design from cw32-hal2 (MIT OR Apache-2.0; attribution
statement in its `THIRD_PARTY.md`) (source: embassy-cw32 README "Related
work"; CHANGELOG 0.2.0).

## B.5 Version timeline (key milestones)

- **2026-01-17**: probe-rs v0.31.0 ships built-in targets for
  CW32L010/L011/L012 (PR #3682) (source: status table in the
  cw32-probe-rs-compat README).
- **2026-06-25**: the svd2rust-generated `cw32l012_pac` 0.1.1 lands on
  crates.io (a name squat unrelated to this book's ecosystem) (source:
  cw32l012-pac README).
- **2026-10-17 to 10-24**: the concentrated period of cw32l012-hal hardware
  verification (TESTING.md records the first examples from 2026-10-17
  through the ecosystem examples on 10-24); embassy-cw32 0.1.0/0.2.0
  released (2026-10-24); the cw32l012-pac `rt` smoke test the same day
  (source: TESTING.md; embassy-cw32 CHANGELOG; cw32l012-pac README).
- **2026-10-31**: the CW32 detection layer goes upstream (PR #4598 merged,
  #4550 closed); cw32-detect builds against probe-rs 0.32.0 with zero
  warnings and passes on-board identification testing (source: the top note
  and "板上验证" [on-board verification] in the cw32-probe-rs-compat README).
  The F030 target PR #4549 has been merged upstream (for the merge date,
  defer to the probe-rs repo; source: cw32-bsp README ecosystem diagram,
  "both merged upstream").
- **Pending**: the crates.io release slot for cw32-pacs; republishing
  `cw32l012-pac` under a new name; scheduling embassy-cw32 0.4.0 (together
  with the DMA/RTC wrap-up) (source: each repo's README/CHANGELOG).
