//! 第 2 章例程：PAC 裸寄存器点灯——GPIOC 门控（KEY 写保护）→ PC13
//! 模拟态转输出 → TOG 翻转 + 软件延时。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch02_blink -p cw32-book-pac
//! 状态：仅编译未上板；寄存器路径与 cw32l012-hal 实测例 blink_pc13 同源
//! （TESTING.md：KEY 门控→ANALOG 清零→TOG，PC13 500ms 方波）。

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use cw32l012_pac_um::{Peripherals, sysctrl as sc};

/// PC13 在端口寄存器中的位掩码（GPIO 位段均按 PINy 编号：PIN13 = bit13）
const PIN13: u32 = 1 << 13;

/// 粗略的软件延时（复位默认 HCLK = 4 MHz 时约 0.5~1 秒量级，未精确标定）。
/// 用 volatile 计数是防止空转循环被优化掉——不带 volatile，
/// release 构建会直接删掉这个循环。
fn delay(n: u32) {
    let mut left = n;
    // SAFETY: 只在本函数内经裸指针 volatile 访问栈上局部变量，单核无竞争
    unsafe {
        let p = core::ptr::addr_of_mut!(left);
        while p.read_volatile() > 0 {
            p.write_volatile(p.read_volatile() - 1);
        }
    }
}

#[entry]
fn main() -> ! {
    // 外设单例：Peripherals::take 只能成功一次（返回 None 即已被取走）
    let p = Peripherals::take().unwrap();

    // ---- 第 1 步：打开 GPIOC 的时钟（SYSCTRL_AHBEN，手册 §4.7.11）----
    // KEY 写保护：AHBEN 高 16 位必须写 0x5A5A，否则整个写入无效（PAC
    // sysctrl::KEY = 0x5A5A << 16）；KEY 位读出恒为 0，所以改位前先读
    // 低 16 位现状、写入时把 KEY 原样补回（与 cw32l012-hal rcc::set_bits
    // 同一步序）。
    let v = p.sysctrl.ahben.read() & 0xFFFF;
    p.sysctrl
        .ahben
        .write(sc::KEY | v | sc::AHBEN_GPIOC); // AHBEN.GPIOC = bit6（§4.7.11）

    // ---- 第 2 步：PC13 从模拟态切回数字（GPIOC_ANALOG，手册 §9.6.7）----
    // 复位默认 GPIOx_ANALOG = 0xE000（GPIOC），PIN13 = 1（模拟功能），
    // 数字输入输出被断开，点灯前必须清零（§9.4.1 步骤 1）。
    p.gpioc.analog.modify(|v| v & !PIN13);

    // ---- 第 3 步：方向设为输出（GPIOC_DIR，手册 §9.6.1）----
    // 注意与多数 MCU 相反：DIR.PINy = 1 是输入，0 是输出（PAC gpio::
    // DIR_OUTPUT = 0）；GPIOC 复位值 0xE000，PC13 默认输入，这里清位
    // （§9.4.1 步骤 2）。
    p.gpioc.dir.modify(|v| v & !PIN13);

    loop {
        // ---- 第 4 步：翻转输出（GPIOC_TOG，手册 §9.6.17）----
        // TOG 是只写寄存器：写 1 的位，对应 ODR 位被硬件原子翻转
        // （§9.3），不用读-改-写。
        p.gpioc.tog.write(PIN13);
        delay(500_000);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
