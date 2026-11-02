//! # cw32-book-pac
//!
//! 第 1~3 章例程的共享封装层。第 2 章正文"工程化：从例程到自己的库"
//! 一节记录了这个 lib 的诞生过程：panic handler、软件延时、点灯寄存器
//! 序列这些"每个例程都要抄一遍"的样板，从第二个例程开始就该收进库里。
//!
//! - [`board`]：SysTick 忙等延时（`delay_ms`/`delay_us`）与 PC13 板载
//!   LED（[`board::Led`]），寄存器路径与 `ch02_blink.rs` 逐行同源。
//! - panic handler **不在库里**：第 2 章的教学主线就是"no_std 固件为
//!   什么必须自己提供它"，故 `#[panic_handler]` 仍由各例程自带（第 3 章
//!   起换成 panic-probe；`examples/hal`、`examples/embassy` 两个 crate
//!   则把 panic handler 收进了各自的 lib.rs——见对应章节）。
//!
//! `board` 模块只在 ARM 裸机目标下存在（cortex-m 平台代码在宿主机上
//! 编译不过），宿主构建时本 crate 是一个空壳。

#![no_std]

#[cfg(all(target_arch = "arm", target_os = "none"))]
pub mod board;
