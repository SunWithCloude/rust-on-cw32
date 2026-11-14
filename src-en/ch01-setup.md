# Chapter 1 Environment Setup

In this chapter you will learn:

- to install the Rust stable toolchain with `rustup`, and add the cross-compilation target that microcontrollers need, `thumbv6m-none-eabi`;
- to install the flashing and debugging tool probe-rs, and get to know the debugger in your hand;
- to set up an editor with `rust-analyzer`;
- how the three ecosystem dependencies are declared — exact-pinned crates.io releases (all three dependencies use version refs) — and that **getting through this book requires cloning no repositories**;
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

## The Ecosystem Dependencies: Three Declarations, No Cloning Required

The three ecosystem dependencies of this book's examples (introduced one by one from Chapter 4 on) come from the following sources, with these version conventions:

| Dependency | Role | Version and how it is pulled in |
|------|------|----------------|
| `cw32l012-pac-um` | Register access layer (PAC) | crates.io, exact-pinned `=0.3.0` |
| `cw32l012-hal` | Blocking HAL (the crate lives in a same-named subdirectory of the repo) | crates.io, `"0.9.2"` |
| `embassy-cw32` | Async HAL | crates.io, exact-pinned `=0.4.0` |

**Conclusion first: getting through this book requires cloning no ecosystem repository.** All three dependencies are fetched by cargo automatically. Once Rust is installed, you can start your own project like this from any directory:

```bash
cargo new my-first-cw32
cd my-first-cw32
cargo add cw32l012-pac-um
```

`cargo add cw32l012-pac-um` drops the register access layer into your `Cargo.toml` — the PAC is a zero-dependency, pure register layer, one command and you are done. (About the `-um` suffix in the name: on crates.io, `cw32l012_pac` has been taken by a third-party svd2rust version with defects; the hand-written transcription was published under a new name — the story is in Appendix B.)

The dependency declarations of the three example crates (`examples/pac`, `examples/hal`, `examples/embassy`) are exactly these three lines (taken apart from Chapter 2 on):

```toml
# examples/pac/Cargo.toml
cw32l012-pac-um = { version = "=0.3.0", features = ["rt"] }

# examples/hal/Cargo.toml
cw32l012-hal = "0.9.2"

# examples/embassy/Cargo.toml
embassy-cw32 = { git = "https://github.com/SunWithCloude/Cw32l012_embassy", branch = "main", features = ["rt"] }
```

Two conventions to note:

- **Why the PAC is exact-pinned with `=`**: any "automatic upgrade" of the register layer could silently shift bitfield assumptions (one bit off, and the whole firmware's behavior changes). Upgrades must change the version number by hand and re-run every example. The HAL can use normal semver (`0.9.2`). embassy-cw32 0.4.0 is published on crates.io and is exact-pinned the same way (the book uses `=0.4.0`).
- **For developers who want to modify the ecosystem sources**: clone the ecosystem repos anywhere you like, then add a `[patch.crates-io]` section to **your project's root `Cargo.toml`** pointing the crates.io dependencies back at the local checkouts (this is how this book's own writing-period verification loop worked):

```toml
[patch.crates-io]
cw32l012-pac-um = { path = "../cw32l012-pac" }
cw32l012-hal = { path = "../cw32l012-hal/cw32l012-hal" }
embassy-cw32 = { path = "../embassy-cw32" }
```

Change a line of source, recompile the example, and the effect is immediate — that workflow is beyond a read-only version dependency, but it is an advanced move; none of the book's examples rely on it.

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
- **Writing the dependency as `cw32l012-pac` (without `-um`)**: cargo fetches the same-named third-party svd2rust crate on crates.io, which has defects (mismatched API, three register defects). This ecosystem's hand-written PAC is named `cw32l012-pac-um` — see the naming story in Appendix B.
- **The first build stalls fetching the crates.io index**: resolving the three ecosystem dependencies needs network access (crates.io only — no GitHub clones anymore). On offline machines, run `cargo fetch` once on a networked one.
- **The first build is slow**: that is normal — the three ecosystem dependencies are being fully compiled for the M0+ once; after that, incremental builds finish in seconds.
- **rustup installed inside WSL, probe-rs installed on Windows (or the other way around)**: the two environments cannot see each other; install the full set on one side.
