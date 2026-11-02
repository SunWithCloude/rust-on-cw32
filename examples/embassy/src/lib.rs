//! # cw32-book-embassy
//!
//! 第 7~14 章异步例程的共享封装层。
//!
//! - panic handler 定义在本 lib 根：`#[panic_handler]` 在整个固件里必须
//!   唯一，收进库后各例程**不再自带**——例程里一行 `use cw32_book_embassy
//!   as _;` 就把 lib 拉进链接、继承它。这正是 `panic-probe` 的同款机制：
//!   该 crate 的全部内容也是一个 panic handler，靠 `use panic_probe as _;`
//!   生效（功能经链接符号注入，不需要显式调用任何 API）。
//! - [`board`]：PC13 板载 LED 的最小封装，收敛"门控 → 构造"两行样板。
//!
//! init() 的调用**有意**不在库里包一层：`embassy_cw32::init` 必须是
//! `#[main]` 函数第一行（第 7 章常见坑第一条），把它藏进 helper 只省
//! 一行代码、却把纪律藏了起来。

#![no_std]

pub mod board;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
