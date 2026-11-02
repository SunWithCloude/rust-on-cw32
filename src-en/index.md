# Preface

**Chinese edition**: [/](/) (switch to the Chinese edition)

This book teaches embedded Rust on the **CW32 series of microcontrollers from CMSemicon (Wuhan Xinyuan)**, taking the **CW32L012** (Arm Cortex-M0+, 64 KB Flash / 8 KB SRAM) as the teaching vehicle — from lighting a single LED all the way to the full family of async peripherals.

## Why the CW32L012

- **Small, and real**: an M0+ core with 64K/8K of memory gives exactly the learning curve of a "first microcontroller" — yet the chip carries real peripherals such as DMA, a dual ADC, DAC, RTC, and watchdogs, so it can do actual work once you have learned it.
- **A complete Rust ecosystem chain**: a register access layer (`cw32l012-pac`, transcribed item by item from User Manual V1.4), a blocking HAL (`cw32l012-hal`, zero dependencies with its own runtime), and an async HAL (`embassy-cw32`, the embassy executor plus a SysTick time driver). All three layers star in this book, and all three are real open-source projects under [SunWithCloude](https://github.com/SunWithCloude).
- **Every example compiles**: every piece of code in the book corresponds to a real example file under `examples/` in this repository, kept compiling by CI on `thumbv6m-none-eabi`. Examples verified on hardware are labeled as such; examples that only compile and have not run on a board are labeled as such too — no fudging.

## What You Need

- A **CW32L012** development board (the book's examples assume the on-board LED is on **PC13** and UART1 runs on **PA02(TX)/PA03(RX)** — if your pins differ, change two lines according to your schematic)
- An **ST-Link** (or any CMSIS-DAP/DAP-Link) debugger, wired over SWD with four lines (SWDIO, SWCLK, GND, usually plus 3V3)
- A computer running Windows/Linux/macOS, with a free USB port
- Only two pieces of software, installed in Chapter 1: the **Rust stable toolchain** (including the `thumbv6m-none-eabi` cross-compilation target) and **probe-rs** (flashing, debugging, and collecting RTT logs; recognizing the CW32L012x8 requires 0.33 or newer)

## How to Read This Book

- **Part 1 (Chapters 1–3)** depends on no HAL: install the toolchain, light an LED through the PAC's raw registers, then use RTT to have the chip report to your computer. Understanding this part means understanding what every later abstraction is built on.
- **Part 2 (Chapters 4–6)** gets you productive with the blocking `cw32l012-hal`: one-line LED control, serial I/O, and a quick tour of timers/DMA/RTC/watchdogs. Its style is the closest to a C standard library — well suited for getting things done fast.
- **Part 3 (Chapters 7–9)** enters the async world of embassy: the executor, the time driver, interrupt binding. From here on, waiting no longer occupies the CPU.
- **Part 4 (Chapters 10–14)** goes deeper into async peripherals: PWM, ADC/DAC, DMA, RTC, I2C/SPI — one complete, runnable example per chapter.
- **The appendices** are the toolbox: a porting matrix for the 11 family, an ecosystem overview, a troubleshooting guide, and a probe-rs quick reference. When you get stuck, reach for Appendix C first.

You do not have to read it front to back: Part 1 is best taken in order; after that every chapter stands alone, so fill in whatever you are missing.

## Conventions

- Examples are numbered by chapter: `examples/<layer>/examples/chNN_name.rs`, and each chapter opens with its build command.
- Registers and bit fields are always annotated with the user manual section (e.g. §25.4); "the manual" means *CW32L012 User Manual* V1.4. Where a page-by-page cross-check has not been done, the section reference follows the comments in the PAC/HAL source and is labeled honestly.
- Command lines are given in bash style. Commands with path arguments need attention to separators under Windows PowerShell; commands without paths are verbatim identical in both shells — each such case is noted explicitly.
- All examples in this book use the Rust 2024 edition; constructs like `#[unsafe(no_mangle)]` are edition-2024 requirements, and the text explains them where they appear.

---

This book and the ecosystem repositories keep evolving. If you find a problem, please open an issue at [the repository's issue tracker](https://github.com/SunWithCloude/rust-on-cw32/issues).
