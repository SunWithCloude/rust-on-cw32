//! 第 3 章例程：RTT 日志——defmt 帧经 SEGGER-RTT 上行通道送调试器，
//! `probe-rs run` 原生解码，不占用任何 UART 引脚。
//!
//! 延时与心跳灯改用第 2 章工程化小节诞生的 `board` 封装
//! （`board::delay_ms` / `board::Led`）：日志打点与 LED 翻转同步，
//! 板上首跑时"灯在闪而日志没滚出来"即 RTT 链路问题。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch03_rtt -p cw32-book-pac
//! 状态：仅编译未上板。依赖组合（defmt 1 + defmt-rtt 0.4 +
//! panic-probe(print-defmt)）与 embassy-cw32 的 rtt_hello 完全一致，
//! 后者提交记录的验证级别为 "release link clean"（链接通过）。

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m_rt::entry;
use cw32_book_pac::board::{Led, delay_ms};
use cw32l012_pac_um::Peripherals;
use defmt_rtt as _; // 全局 logger：defmt 帧写入 RTT 上行通道
use panic_probe as _; // panic handler：panic 时经 defmt 报告位置后停机

// defmt 1.x 用 timestamp! 宏定义"时间戳提供者"：宏在顶层展开为一个
// #[export_name = "_defmt_timestamp"] 的函数，defmt-rtt 不提供它，链接
// 就靠这里。defmt 0.3 时代是手写 #[no_mangle] fn _defmt_timestamp() -> u64，
// defmt 1.x 起签名改为接收 Formatter、由本宏代写。
// 本例没有时间驱动，时间戳就用"这是第几条日志"的序号——defmt 只负责把
// 数字原样带给主机。要真正的毫秒 uptime 需要时间源（embassy 版 rtt_hello
// 用 embassy-time 提供，本书第 7 章后即是此路线）。
static SEQ: AtomicU32 = AtomicU32::new(0);
defmt::timestamp!("{=u32}", SEQ.load(Ordering::Relaxed));

// defmt-rtt 用 critical-section 保护日志缓冲，而裸机 M0+ 没有现成的
// critical-section 实现，必须由固件自己提供一个，否则链接报
// `undefined symbol: _critical_section_1_0_acquire`。单核标准做法：
// 读 PRIMASK → `cpsid i` 关中断；释放时仅当初"中断原本开着"才恢复。
// 与 cortex-m 的 critical-section-single-core、cw32l012-hal 各自的实现
// 同一写法——一个固件只能有一个实现，本书靠例程 crate 彼此隔离。
struct SingleCoreCs;
critical_section::set_impl!(SingleCoreCs);

unsafe impl critical_section::Impl for SingleCoreCs {
    unsafe fn acquire() -> critical_section::RawRestoreState {
        let primask: u32;
        unsafe {
            core::arch::asm!("mrs {0}, PRIMASK", "cpsid i", out(reg) primask);
        }
        (primask & 1) as u8 // RawRestoreState = u8（dev-dependencies 里钉死）
    }

    unsafe fn release(token: critical_section::RawRestoreState) {
        if token == 0 {
            unsafe { core::arch::asm!("cpsie i") };
        }
    }
}

#[entry]
fn main() -> ! {
    // 引用 PAC：让向量表随 rlib 进入链接（PAC docs/rt-smoke.md 的纪律）
    let p = Peripherals::take().unwrap();

    // 心跳灯：门控/退模拟/方向三步在 board::Led::new 里（第 2 章工程化）
    let mut led = Led::new(&p);

    defmt::info!("ch03 RTT hello — 复位默认时钟 HSI 4 MHz");

    loop {
        // M0+ 没有原子"读-改-写"指令，core 的 fetch_add 在 thumbv6m 上
        // 不存在——只能 load/store 三步走；单核且尚未开中断，安全。
        let n = SEQ.load(Ordering::Relaxed) + 1;
        SEQ.store(n, Ordering::Relaxed);
        defmt::info!("heartbeat #{=u32}", n);
        led.toggle(); // 灯与日志打点同步，双通道互为佐证
        delay_ms(500);
    }
}
