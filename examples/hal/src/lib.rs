//! # cw32-book-hal
//!
//! 第二部分（第 4~6 章）例程的共享封装层。
//!
//! HAL 本身（`cw32l012-hal`）不提供 panic handler——`#[panic_handler]`
//! 在整个固件里必须唯一，这是应用侧的决定。本书把这个决定做一次：
//! handler 定义在本 lib 根，各例程一行 `use cw32_book_hal as _;` 把
//! lib 拉进链接、继承它。这正是 `panic-probe` 的同款机制（功能经链接
//! 符号注入，不需要显式调用任何 API）。GPIO/延时等 HAL API 教学
//! 内容仍在各例程里，本 lib 不做二次封装。

#![no_std]

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
