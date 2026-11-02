# Chapter 1 Environment Setup

In this chapter you will learn:

- to install the Rust stable toolchain with `rustup`, and add the cross-compilation target that microcontrollers need, `thumbv6m-none-eabi`;
- to install the flashing and debugging tool probe-rs, and get to know the debugger in your hand;
- to set up an editor with `rust-analyzer`;
- the directory layout shared by this book's repository and the three ecosystem repositories, and why the `path` dependencies are wired the way they are;
- to verify the whole environment with one command: compile every example in the book into Cortex-M0+ firmware.

This chapter has no firmware example — the goal is that by the end, Chapter 2's `ch02_blink` compiles with a single command.

## Installing Rust

Rust is installed and managed by [`rustup`](https://rustup.rs); the entry point is the same on all three platforms:

```bash
# Windows: download and run rustup-init.exe (linked from the https://rustup.rs front page)
# Linux / macOS:
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

On Windows, stepping through `rustup-init.exe` with Enter is fine; if it prompts that Visual Studio Build Tools are missing, follow its guidance and install them (the MSVC target needs them as the linker).

After installing, open a new terminal and confirm the version:

```bash
rustc --version
```

While writing this book we used the 1.97 series of stable. **You do not need the newest** — but please stay on the stable channel: every example in this book relies on stable features only, and the Rust 2024 edition requires 1.85 or later.

## Adding the Cross-Compilation Target

By default, `rustup` installs a toolchain that compiles for your own machine only. The microcontroller is an Arm Cortex-M0+ core (the ARMv6-M architecture), which needs its own compilation target:

```bash
rustup target add thumbv6m-none-eabi
```

This command drops a copy of the `thumbv6m-none-eabi` core libraries (precompiled `core` and friends) into the toolchain. How to read the name: `thumb` is ARM's compressed-instruction execution state, `v6m` corresponds to ARMv6-M (Cortex-M0/M0+), and `none-eabi` means "no operating system, embedded ABI". The whole book compiles for this one target and nothing else.

## Installing probe-rs

[probe-rs](https://probe.rs) is this book's flashing and debugging backend: writing firmware, resetting the chip, collecting RTT logs (Chapter 3), and breakpoint debugging all go through it. It directly supports the common debug probes — ST-Link, DAP-Link, J-Link and more.

Pick one installation route for your platform (full details in the [official installation documentation](https://probe.rs/docs/getting-started/installation/)):

```bash
# All platforms: build and install from source (a few minutes; always pass --locked)
cargo install probe-rs-tools --locked

# Windows: download and run an installer from https://github.com/probe-rs/probe-rs/releases
# macOS / Linux: brew install probe-rs-tools
# If you already have cargo-binstall: cargo binstall probe-rs-tools
```

Support for the CW32L012 (target chip name `CW32L012x8`) works out of the box in probe-rs **0.33 and newer**. Confirm it after installing:

```bash
probe-rs --version
```

Connect the debugger to the board (the four SWD wires: SWDIO, SWCLK, GND, usually plus 3V3 for power), plug it into the computer, then run:

```bash
probe-rs list
```

If the output lists your probe (an ST-Link V2 shows up as `0483:3748`), the USB path works. For troubleshooting a probe that cannot be found, see Appendix D.

## The Editor

Any editor can write Rust, but **rust-analyzer is a must** — it is far more than an autocomplete plugin. Without it you have no go-to-definition, no type hints, no register-name completion, and writing this book's raw PAC register code becomes genuinely painful.

- **VS Code** (recommended): installing the official `rust-analyzer` extension is enough; no extra configuration needed.
- Other editors: rust-analyzer is an LSP server; Vim/Emacs/Neovim all have mature integrations — see the [rust-analyzer manual](https://rust-analyzer.github.io/manual.html).

Optional: the **probe-rs debugging** extension for VS Code adds graphical breakpoint debugging — flashing, breakpoints, and variable inspection all happen inside the editor. Chapters 2 and 3 only need the command line; installing this extension any time from Chapter 7 onward is fine too.

## Repository Layout: One Book with Three Ecosystem Repos

The book's examples do not bundle their dependencies; they reference the sources of three ecosystem repositories directly:

| Repository | Role | How the book references it |
|------|------|--------------|
| `cw32l012-pac` | Register access layer (PAC) | the `examples/pac` example crate |
| `cw32l012-hal` | Blocking HAL (the crate lives in a same-named subdirectory of the repo) | the `examples/hal` example crate |
| `embassy-cw32` | Async HAL | the `examples/embassy` example crate |

They are expected to sit **side by side** with the book's repository (in the example below, `book/` stands for any directory where you keep code):

```text
book/
├── rust-on-cw32/          # this book's repository
│   └── examples/pac/Cargo.toml
├── cw32l012-pac/          # PAC: the repository root is the crate
├── cw32l012-hal/          # HAL: a same-named crate directory under the repo root
│   └── cw32l012-hal/
└── embassy-cw32/          # async HAL: the repository root is the crate
```

Clone them into these positions:

```bash
cd book
git clone https://github.com/SunWithCloude/rust-on-cw32
git clone https://github.com/SunWithCloude/cw32l012-pac
git clone https://github.com/SunWithCloude/cw32l012-hal
git clone https://github.com/SunWithCloude/embassy-cw32
```

## Why the path Dependencies Are Wired This Way

Open `examples/pac/Cargo.toml` and the dependency reads:

```toml
[dependencies]
cw32l012-pac = { path = "../../../cw32l012-pac", features = ["rt"] }
```

`../../../` walks up three levels from `examples/pac/` and lands exactly on `book/` — the directory where the three ecosystem repos sit side by side. Cargo resolves `path` dependencies **relative to the Cargo.toml they are written in**, so as long as the side-by-side layout is kept when cloning, everything works immediately, with no extra installation of any ecosystem component.

Two reasons behind this design:

- **Change a line, see it at once.** This book explains PAC/HAL implementation details at length (Chapter 2, for instance, dissects the KEY write-protection write sequence line by line). Add a comment or a marker in the ecosystem source, recompile the example, and you can observe the effect — a dependency published to crates.io cannot do that.
- **The version always follows the repository.** The three ecosystem repos evolve fast (the HAL is still gaining peripherals at a brisk pace). A path dependency keeps "the code in the book" and "the code in the repository" the same code; there is no mismatch where "the book shows the v0.9 API while crates.io already serves v1.0".

The price is that the clone location must be right. If you put an ecosystem repo somewhere else, the build fails with `failed to load source for dependency ... cw32l012-pac` — move the directory back beside the book, or change the `path` in that example crate's `Cargo.toml` to your actual location.

## Acceptance: Compile Every Example in One Command

Run this from the **root of the book's repository**:

```bash
# bash (Linux/macOS/Git Bash)
cargo build --workspace --target thumbv6m-none-eabi --release --examples
```

This command has no path arguments, so under PowerShell it is identical verbatim — copy it as is (only the prompt differs):

```text
PS book\rust-on-cw32> cargo build --workspace --target thumbv6m-none-eabi --release --examples
```

Taking the command apart piece by piece:

- `--workspace`: build all three example crates in the repository (`examples/pac`, `examples/hal`, `examples/embassy`);
- `--target thumbv6m-none-eabi`: cross-compile for the Cortex-M0+, not for your own machine;
- `--release`: an optimized build (firmware size and speed both at release standards);
- `--examples`: build the example files under each crate's `examples/` directory.

The first run also compiles the ecosystem crates and takes on the order of minutes; when it ends with one `Finished` line after another and no errors, the environment is fully ready. To build a single chapter's example (the form each chapter of this book gives), scope it to the crate with `-p`:

```bash
cargo build --release --target thumbv6m-none-eabi --example ch02_blink -p cw32-book-pac
```

> To also build this book itself as a web book (optional): `cargo install mdbook`, then run `mdbook build` at the repository root.

## Common Pitfalls

- **Forgot to run `rustup target add`**: the build fails with `can't find crate for 'core'` — the target's core libraries are missing; go back to the second section and install them.
- **`cargo install probe-rs-tools` without `--locked`**: dependency resolution may fail; the official documentation explicitly requires `--locked`.
- **Ecosystem repos not placed side by side**: the build reports `failed to load source for dependency`; move the directories per the layout above, or fix the `path`.
- **The first build is slow**: that is normal — the three ecosystem crates are being fully compiled for the M0+ once; after that, incremental builds finish in seconds.
- **rustup installed inside WSL, probe-rs installed on Windows (or the other way around)**: the two environments cannot see each other; install the full set on one side.
