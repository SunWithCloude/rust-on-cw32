# 第 6 章 阻塞侧速览：定时器、DMA、RTC 与看门狗

前两章的外设（GPIO、UART）都是"CPU 发号施令"。这一章换一批角色：
**定时器**自己数拍子、**DMA** 替 CPU 搬数据、**RTC** 在主时钟停摆时继续
走时、**看门狗**盯着程序别跑飞。四个外设一章讲完是不可能的——本章的
目标是把每个外设的"最小可用面"各跑通一个例程，混个脸熟；深水区留给
第四部分（第 10 章 PWM、第 12 章 DMA、第 13 章 RTC 与看门狗）。

**这一章你会学到：**

- BTIM 基本定时器的周期溢出与更新中断：PSC/ARR 怎么自动选、ISR 怎么挂；
- DMA 存储器→存储器搬运：一个 `mem_to_mem` 调用背后的通道与触发；
- RTC 的时钟源选择（LSE 优先、LSI 兜底）与 BCD 日期时间的读写；
- IWDT 的超时复位循环与键值机，以及 WWDT"窗口喂狗"的概念；
- CW32 特色的 `R1W0` 清标志写法（写 0 才清，不是写 1）。

## 6.1 定时器：让硬件数拍子

BTIM（基础定时器，手册 §14）是三个定时器家族里最简单的：16 位自动
重装载计数器 + 16 位预分频，计数时钟直接取 PCLK——**没有 STM32 式的
×2 规则**（手册图 14-5）。溢出频率的公式：

```text
溢出频率 = PCLK / ((PSC+1) × (ARR+1))
```

HAL 的 `Btim` 把选分频这件事接管了（摘自 cw32l012-hal `src/btim.rs`
的 `calc_period`）：策略是**选最小 PSC 使 ARR ≤ 0xFFFF**——ARR 越大，
量化误差相对越小。2 Hz @ 4 MHz 的解有单元测试背书：PSC=30（÷31）、
CK_CNT ≈ 129.0 kHz、ARR=64515，实际溢出频率 1.99997 Hz。

启动与挂中断只需四步（对应本章例程的段 A）：

```rust,ignore
let mut tim = Btim::new(p.btim1, &mut rcc);          // 自动开 APBEN2[2] 门控
tim.start_periodic(clocks.pclk_hz, 2)?;              // 配 PSC/ARR、清残留标志、启动
tim.enable_update_irq(true);                         // 外设层开更新中断（DIER.UIE）
unsafe { runtime::Interrupt::Btim1.enable(); }       // NVIC 层放行 IRQ20
```

注意中断有**两层开关**：`start_periodic` 打开的是外设内部的 `DIER.UIE`，
NVIC 层的 `Interrupt::Btim1`（IRQ20，向量符号 `BTIM1_irq`）还要单独
`enable()`，最后 `cpsie i` 全局开中断。漏掉任何一层，中断都到不了 ISR。
挂上中断后，主循环就可以 `wfi` 睡觉——CPU 停在睡眠态，定时器溢出时
硬件自动唤醒进 ISR，醒来再睡。同样的闪烁，忙等版（第 4 章）CPU 一直在
空转计数，中断版几乎不耗电。

ISR 本身用 HAL 的 `interrupt!` 宏声明（6.5 节例程里有完整用法）：宏只做
一件事——给中断处理函数挂上向量表符号名 `BTIM1_irq`（链接脚本
`memory.x` 里每个向量都是 `PROVIDE(XXX_irq = DefaultHandler)` 的弱默认，
同名强符号自动覆盖它）。ISR 里要做两件例行公事：**清标志**、**干活**。
清标志又撞上 CW32 的招牌脾气：

> **`ICR` 是 R1W0 寄存器**（复位值 0xFFFF_FFFF）：写 1 无动作、**写 0 才
> 清**。清更新标志要写 `ICR_CLEAR_UIF = !1`——把 bit0 置 0、其余位全 1。
> 从 STM32 带来的"写 1 清标志"肌肉记忆在这里会把标志永远留着，中断
> 一退出又立刻重进。

顺带一提另外两个定时器家族：GTIM（通用，4 实例 × 4 通道 PWM）和
ATIM（高级，互补 PWM + 死区 + 刹车）。生态仓的 `gtim_pwm` 例程已在
实板上验证过 1 kHz PWM 呼吸（TESTING.md：ARR=3999/PSC=0，CNT 铺满
0..3999）；PWM 的正题在第 10 章。

## 6.2 DMA：搬数据不用 CPU

DMA（手册 §8）有 4 条独立通道，通道号越小优先级越高（§8.5.4）。一次
传输按手册 §8.4.1 的 14 步流程配置，HAL 把它收进三个调用：

```rust,ignore
let mut dma = Dma::new(p.dma, &mut rcc);   // 自动开 AHBEN[0] 门控
let ch = dma.channel(3);                   // 通道 3
ch.configure(&cfg);                        // DISABLE → 清标志 → 写寄存器 → EN
ch.start();                                // 软件触发（TRIG.SOFTSRC）
ch.wait()?;                                // 阻塞等传输完成
```

例程里用的 `mem_to_mem` 是这条路的打包版：泛型按元素大小自动选位宽
（u32 → 32 位）、自动选通道 3、软件触发、阻塞等待，传输完成后逐字
比对即可验收——**CPU 只在开头发令、结尾验收，中间的搬运全程不参与**。
真正的价值在硬件触发：把触发源换成 ADC 转换完成（EOC），每个样本
就自动落进内存，这是第 12 章的主菜。生态仓 `dma_demo` 的 P2 相位
就是这么跑的：DAC→ADC 片上回环，DMA 搬 256 个 16 位结果，实测
`P2_REMAIN=0`、`P2_STATUS=5`（表 8-2「传输完成」）。

除了 R1W0，DMA 还有两个容易踩的硬件行为（`dma.rs` 文档里记了全账）：

- **`CNT.REPEAT` 每轮传输后被硬件清 0**（§8.5.2）——连续多轮传输必须
  每轮重新 `configure()`，只改计数不重配是无效的；
- **软件触发 `SOFTSRC` 完成后自动清零**——再传一次要重新写。

## 6.3 RTC：主时钟睡了它也在走

RTC（手册 §13）是 BCD 日历：年月日周 + 时分秒，硬件自动处理闰年与
大小月。它挂在独立的时钟域上，可选 LSE（外部 32.768 kHz 晶体）、
LSI（内部 ~32.8 kHz RC）等四个源（`RtcClock` 枚举）。`Rtc::new` 的
约定是"你要的源起振失败就返回 Err"，由调用方决定兜底方案——例程
采用 LSE 优先、LSI 兜底：

```rust,ignore
let rtc = match Rtc::new(p.rtc, &mut rcc, RtcClock::Lse) {
    Ok(r) => r,
    Err(_) => match Rtc::new(p.rtc, &mut rcc, RtcClock::Lsi) {
        Ok(r) => r,
        Err(_) => loop { core::hint::spin_loop() },
    },
};
```

LSE 判定自带约 2 s 的起振等待（`Rcc::enable_lse` 忙等 `LSE.STABLE`，
典型起振 200~500 ms）；失败时 RTC 配置尚未动过，退回 LSI 是干净的。
实测（TESTING.md `rtc_demo`，2026-10-23）本板 **LSE 未起振（CLK=2 →
走 LSI）**——StartKit 没焊 32.768 kHz 晶体或负载电容不匹配。没焊
晶体的板子 RTC 也照跑：LSI 出厂已 TRIM（实测 32768 Hz），只是长期
走时精度没有单独标定，要准点请接 LSE。

日期时间的字段是**十进制值**（模块内部负责转 BCD），`DateTime` 一个
结构体全装下：`year` 0..=99 表示 20xx、`weekday` 0=周日..6=周六。
写用 `set_datetime`（会先做字段合法性检查），启动用 `start()`，读用
`datetime()`（连读两次取稳定值，手册 §13.3.6 的推荐做法）。所有 RTC
寄存器写入都要过**解锁钥匙**：`KEY` 依次写 0xCA、0x53，完事再写
0xFF 重新上锁——HAL 的 `with_unlocked` 助手把这个流程包死了，手写
寄存器时漏掉解锁，写入同样静默丢弃。

一个实测踩出来的坑值得记住（TESTING.md `rtc_demo`）：**停表状态下
`CR1.WAIT` 不会清零**。手册 §13.3.6 的"等 WAIT"流程只适用于**运行中**
写 DATE/TIME；停表后可以直接写。照运行中流程等 WAIT 会一直等到超时
返回 Err——而时间其实已经写进去了。

## 6.4 看门狗：程序跑飞前咬一口

看门狗的逻辑只有一句话：一个倒计数器在跑，你不周期性"喂"它，它
数到零就让芯片复位。程序跑飞（死循环、卡死在等待里）时喂狗代码
不再执行，超时复位把系统拉回正轨。

**IWDT 独立看门狗（§19）**——计数时钟是独立的 RC10K（≈10 kHz），
与主时钟域完全隔离，主时钟崩溃时照样能复位芯片。操作全走**键值机**：
`0xCCCC` 启动、`0xAAAA` 喂狗、`0x5555` 解锁改配置、`0x5A5A`+`0xA5A5`
停止（仅调试期使用）。HAL 的包装把键值藏起来：

```rust,ignore
let mut wdt = Iwdt::new(p.iwdt, &mut rcc);
wdt.start(TimeoutMs(1500))?;   // 超时 ≈1.5s（RC10K 有 ±% 级漂移）
wdt.feed();                    // 喂狗：重载计数值
```

`start` 自动求解分频（4..512 八档）与 12 位 ARR：1500 ms → 分频 /4、
ARR=3749，名义超时正好 1.5 s；因为 RC10K 本身有百分之几级漂移，
`TimeoutMs` 给的是近似值，安全设计要留余量——例程的喂狗间隔
600 ms < 超时的一半。

**WWDT 窗口看门狗（§20）**——在 IWDT 之上加了"**不能喂得太早**"：
7 位递减计数器，喂狗必须落在窗口 [WINR, WCNT] 内（§20.3.4），过早
喂狗同样触发复位。这防的是另一种事故：程序没死，但跑乱了节奏
（比如某个循环提前返回、多跑了一遍），固定间隔喂狗照样能喂上，
窗口喂狗却会把节奏异常暴露出来。HAL 的 `Wwdt::feed()` 会先读当前
计数值、**落在窗口内才写**——过早时返回 `Err(FeedError::TooEarly)`
而不是让芯片复位；`feed_forced()` 才是真实写入，用作对照实验。
生态仓 `wwdt_demo` 的实测（TESTING.md，2026-10-17~19）：窗口
[98.3, 200.7] ms，50 轮里所有"过早喂狗"全部被拦截（过早时刻的
`LAST_CNT=0x64` > WINR=0x58，判定正确），正常喂狗全部成功，
程序零复位。本章例程只上 IWDT，WWDT 留给第 13 章。

## 6.5 例程：两个零接线文件

四个外设装进两个例程，都不需要接线。构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch06_tim_dma -p cw32-book-hal
cargo build --release --target thumbv6m-none-eabi --example ch06_rtc_iwdt -p cw32-book-hal
```

**ch06_tim_dma.rs**——段 A 是 BTIM1 2 Hz 更新中断闪灯（主循环 `wfi`），
段 B 是 DMA 存储器→存储器搬运 16 个 u32 并逐字校验，结果落在
`DMA_OK`/`DMA_BAD` 静态量（调试器可读）：

```rust
//! 第 6 章例程（上）：BTIM1 定时中断闪灯 + DMA 存储器搬运（零接线）。
//!
//! 段 A（定时器）：BTIM1 溢出中断 2Hz，ISR 里 TOG 翻转 PC13，主循环
//! `wfi` 睡眠——与 cw32l012-hal examples/timer_blink.rs 同源。
//! 段 B（DMA）：`Dma::mem_to_mem` 把 16 个 u32 从 SRC 搬进 DST 并逐字
//! 校验，结果落在 `DMA_OK`/`DMA_BAD` 静态量（调试器可读）——搬运路径
//! 与 examples/dma_demo.rs 的 P1 相同（原例为 64 字版）。
//!
//! 构建后现象：PC13 电平每 500ms 翻转一次（溢出 2Hz）= 中断路径活着；
//! DMA 结果需接调试器读静态量。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch06_tim_dma -p cw32-book-hal
//! 状态：同源已验证——两段分别对应 TESTING.md 实测通过表的 timer_blink
//!       （PC13 翻转序列 0,0,1,0,0,1,1,0）与 dma_demo（P1_OK=64、
//!       P1_BAD=0xFFFF_FFFF）条目；本组合文件本身仅编译未上板。

#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, write_volatile};

use cw32l012_hal::{
    btim::Btim,
    dma::Dma,
    gpio::{Level, Pin, PortId},
    pac,
    rcc::Rcc,
    runtime, Peripherals,
};

/// 中断次数计数（调试器可观察）
static mut ISR_COUNT: u32 = 0;
/// DMA 源数据（编译期生成）
static SRC: [u32; 16] = {
    let mut a = [0u32; 16];
    let mut i = 0;
    while i < 16 {
        a[i] = 0xA5A5_0000 ^ (i as u32);
        i += 1;
    }
    a
};
/// DMA 校验：逐字一致的元素数（应 = 16）
static mut DMA_OK: u32 = 0;
/// DMA 校验：首个不匹配下标（0xFFFF_FFFF = 全部一致）
static mut DMA_BAD: u32 = 0xFFFF_FFFF;

#[unsafe(no_mangle)]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };

    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks();

    let _led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);

    // ---- 段 B：DMA 存储器→存储器（先搬运，后开中断）----
    {
        let mut dma = Dma::new(p.dma, &mut rcc);
        let mut dst = [0u32; 16];
        // 阻塞搬完（软件触发、32 位宽、自动选通道 3）；即使 Err 也不提前
        // 退出，让下面的逐字校验把结果如实记进 DMA_OK/DMA_BAD
        let _ = dma.mem_to_mem(&SRC, &mut dst);
        let mut ok = 0u32;
        let mut bad = 0xFFFF_FFFFu32;
        for i in 0..16 {
            if dst[i] == SRC[i] {
                ok += 1;
            } else if bad == 0xFFFF_FFFF {
                bad = i as u32;
            }
        }
        unsafe {
            write_volatile(addr_of_mut!(DMA_OK), ok);
            write_volatile(addr_of_mut!(DMA_BAD), bad);
        }
    }

    // ---- 段 A：BTIM1 @2Hz 更新中断闪灯 ----
    let mut tim = Btim::new(p.btim1, &mut rcc);
    match tim.start_periodic(clocks.pclk_hz, 2) {
        Ok(_) => {}
        Err(_) => loop {
            core::hint::spin_loop();
        },
    }
    tim.enable_update_irq(true);

    // NVIC 层使能 BTIM1（IRQ20），然后全局开中断
    unsafe {
        runtime::Interrupt::Btim1.enable();
        core::arch::asm!("cpsie i");
    }

    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

// BTIM1 更新中断：清标志 → 翻转 PC13（TOG 原子写）→ 计数
cw32l012_hal::interrupt!("BTIM1_irq", btim1_isr, {
    unsafe {
        let p = Peripherals::steal();
        p.btim1.icr.write(pac::btim::ICR_CLEAR_UIF);
        p.gpioc.tog.write(1 << 13);
        ISR_COUNT += 1;
    }
});

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
```

烧录后的现象：PC13 电平每 500 ms 翻转一次——这就是"中断路径活着"的
直接证据（CPU 在 `wfi` 里，灯还在闪，说明是硬件定时器在唤醒它）。
DMA 的搬运结果要用调试器读 SRAM 静态量：`DMA_OK` 应为 16，
`DMA_BAD` 应为 0xFFFF_FFFF（无失配）；`ISR_COUNT` 每秒 +2。

**ch06_rtc_iwdt.rs**——段 A 让 RTC 从 23:59:50 起跑 12 秒，读回应已
跨到次日，一次验证 BCD 进位、跨日与星期递增；段 B 启动 1.5 s 超时的
IWDT，前 5 次喂狗（PC13 快闪 5 下）之后故意停喂，等看门狗咬人复位、
固件从头再跑：

```rust
//! 第 6 章例程（下）：RTC 跨日走时 + IWDT 看门狗复位循环（零接线）。
//!
//! 段 A（RTC）：优先 LSE，不起振则退回 LSI（实测本板 LSE 未起振——
//! 没焊 32.768k 晶体或负载电容不匹配，实跑走 LSI）；设 2026-10-23（周五）
//! 23:59:50，启动后等 12s 读回——应已跨到次日（周六），一次验证 BCD
//! 进位、跨日与星期递增——与 cw32l012-hal examples/rtc_demo.rs 的 P1 同源。
//! 段 B（IWDT）：1.5s 超时；前 5 次喂狗（PC13 每次翻转 → 快闪 5 下），
//! 之后故意停喂 → 1.5s 后看门狗复位、固件从头再跑——与
//! examples/wdt_demo.rs 同源。
//!
//! 上电节奏：前 ~12s 灯不亮（RTC 在做跨日测试）→ 快闪 5 下 →
//! 灭 ~1.5s → 复位、再来一轮。走时结果在 CLK/DT_AFTER/TM_AFTER
//! 静态量（调试器可读）。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch06_rtc_iwdt -p cw32-book-hal
//! 状态：同源已验证——两段分别对应 TESTING.md 实测通过表的 rtc_demo
//!       （CLK=2 走 LSI、跨日 00:00:01、星期递增）与 wdt_demo（BOOT_ID
//!       增至 6 → 复位回 1 循环）条目；本组合文件本身仅编译未上板。

#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, write_volatile};

use cw32l012_hal::{
    delay::Delay,
    gpio::{Level, Pin, PortId},
    rcc::Rcc,
    rtc::{DateTime, Rtc, RtcClock},
    wdt::{Iwdt, TimeoutMs},
    Peripherals,
};

/// 实际使用的 RTC 时钟源（1=LSE、2=LSI、0=失败）
static mut CLK: u32 = 0;
/// 12s 后读回的日期（year<<24 | month<<16 | day<<8 | weekday）
static mut DT_AFTER: u32 = 0;
/// 12s 后读回的时间（hour<<16 | minute<<8 | second）
static mut TM_AFTER: u32 = 0;
/// 本次开机喂狗次数（看门狗复位后重新从 0 爬升）
static mut BOOT_ID: u32 = 0;

fn pack_date(d: &DateTime) -> u32 {
    ((d.year as u32) << 24) | ((d.month as u32) << 16) | ((d.day as u32) << 8) | d.weekday as u32
}

fn pack_time(d: &DateTime) -> u32 {
    ((d.hour as u32) << 16) | ((d.minute as u32) << 8) | d.second as u32
}

#[unsafe(no_mangle)]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };

    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let mut delay = Delay::new(&rcc.clocks());
    let mut led = Pin::new(p.gpioc, PortId::C, 13, &mut rcc).into_output(Level::Low);

    // ---- 段 A：RTC 跨日走时（LSE 优先，失败退 LSI）----
    let rtc = match Rtc::new(p.rtc, &mut rcc, RtcClock::Lse) {
        Ok(r) => {
            unsafe { write_volatile(addr_of_mut!(CLK), 1) };
            r
        }
        Err(_) => match Rtc::new(p.rtc, &mut rcc, RtcClock::Lsi) {
            Ok(r) => {
                unsafe { write_volatile(addr_of_mut!(CLK), 2) };
                r
            }
            Err(_) => loop {
                core::hint::spin_loop();
            },
        },
    };

    // 设 23:59:50 → 启动 → 12s 后应已跨到次日（实测 00:00:01，周六；
    // 名义 :02，差 1s 为 LSI 走时与延时基准的合计偏差，见 TESTING.md）
    let t0 = DateTime {
        year: 26,
        month: 10,
        day: 23,
        weekday: 5, // 0=周日，5=周五
        hour: 23,
        minute: 59,
        second: 50,
    };
    if rtc.set_datetime(&t0).is_err() {
        // 只有字段非法或 RTC 同步异常才会到这里，正常上电不会发生；
        // 冻结在此便于发现问题（LED 保持不动也是信号）
        loop {
            core::hint::spin_loop();
        }
    }
    rtc.start();
    delay.delay_ms(12_000);
    let t1 = rtc.datetime();
    unsafe {
        write_volatile(addr_of_mut!(DT_AFTER), pack_date(&t1));
        write_volatile(addr_of_mut!(TM_AFTER), pack_time(&t1));
    }

    // ---- 段 B：IWDT 看门狗（1.5s 超时；喂 5 次停喂 → 复位循环）----
    let mut wdt = Iwdt::new(p.iwdt, &mut rcc);
    match wdt.start(TimeoutMs(1500)) {
        Ok(_) => {}
        Err(_) => loop {
            core::hint::spin_loop();
        },
    }

    loop {
        unsafe {
            BOOT_ID += 1;
            // 只喂前 5 次；之后故意停喂 → 1.5s 后看门狗复位
            if BOOT_ID <= 5 {
                led.toggle();
                wdt.feed();
            }
        }
        delay.delay_ms(600); // 喂狗间隔 600ms < 1.5s
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
```

上电节奏：前 ~12 s 灯不亮（RTC 在做跨日测试）→ 快闪 5 下 → 灭
~1.5 s → 复位、再来一轮。`BOOT_ID` 记录本次开机以来的喂狗次数，
用调试器采样会看到它 0→5 递增后突然跳回 1——这就是"停喂 → 看门狗
复位"的现场。走时结果在 `CLK`（1=LSE、2=LSI）/`DT_AFTER`/`TM_AFTER`
三个静态量里。

## 6.6 板上验证

**状态：同源已验证（分段）+ 组合文件本身仅编译未上板。** 两个例程
各自的两段分别与生态仓已实测例程同源：

- `ch06_tim_dma` 段 A 与 cw32l012-hal `examples/timer_blink.rs` **逐行
  同源**（仅加注释）；段 B 的搬运路径与 `examples/dma_demo.rs` 的 P1
  相同（16 字精简版，原例 64 字）。
- `ch06_rtc_iwdt` 段 A 与 `examples/rtc_demo.rs` 的 P1 同源（去掉闹钟
  相位）；段 B 与 `examples/wdt_demo.rs` 同源（同一套 BOOT_ID 节奏）。

生态仓对应条目的实测记录（cw32l012-hal `TESTING.md`）：

| 例程 | 验证内容 | 关键读数 | 实测日期 |
|------|----------|----------|----------|
| timer_blink | BTIM1 2Hz 中断 + wfi | PC13 翻转序列 0,0,1,0,0,1,1,0 | 2026-10-17~19 |
| dma_demo | DMA 存储/ADC 硬件触发 | P1_OK=64、P1_BAD=0xFFFF_FFFF；P2_REMAIN=0 | 2026-10-23 |
| rtc_demo | RTC 跨日走时 + 闹钟 | CLK=2（LSI）、跨日 00:00:01、星期递增 | 2026-10-23 |
| wdt_demo | IWDT 超时复位 | BOOT_ID 增至 6 → 复位回 1 循环 | 2026-10-17~19 |
| wwdt_demo | WWDT 窗口喂狗 | EARLY=51/FEEDS=50 同步、LAST_CNT=0x64 被正确拒绝 | 2026-10-17~19 |

"组合文件本身仅编译未上板"的含义要如实交代：本章两个 `.rs` 是把
已验证片段合并后的新文件，**没有以这个组合形态烧过板**。首次上板
按顺序核对：

1. `ch06_tim_dma`：先看 LED 是否 500 ms 翻转（中断路径），再接调试器
   读 `DMA_OK`/`DMA_BAD`（搬运路径），两段各自独立，互不依赖；
2. `ch06_rtc_iwdt`：先读 `CLK` 确认时钟源（本板预期 2=LSI），再等
   12 s 读 `DT_AFTER`/`TM_AFTER`（预期跨日 00:00:01±1 s），最后观察
   BOOT_ID 的复位循环；
3. RTC 读回与预期差 1 s 左右属正常（LSI 走时 + 延时基准的合计偏差，
   见 TESTING.md `rtc_demo` 的说明）；
4. 若 `DMA_OK` < 16：检查是否动了 `mem_to_mem` 的通道号——例程依赖
   HAL 自动选通道 3。

## 6.7 常见坑

- **中断三层开关**：`DIER.UIE`（外设）、NVIC `enable()`（IRQ20）、
  `cpsie i`（全局），缺一层中断都进不来；`wfi` 前三层必须全部就位。
- **R1W0 清标志**：CW32 的 `ICR` 写 0 才清、写 1 无动作（复位值
  0xFFFF_FFFF）。用 STM32 的"写 1 清"习惯会永远清不掉——BTIM 的
  `ICR_CLEAR_UIF = !1`、DMA 的 `icr.write(!(1<<0 | 1<<1))` 都是写 0。
- **ISR 里用 `Peripherals::steal()`**：`take()` 已经在 `main` 里消费过，
  ISR 里只能 `steal()` 再拿一份；同一中断里寄存器冲突由作者自己负责。
- **DMA 的 REPEAT 与 SOFTSRC 都会被硬件自动清零**：连续传输每轮都要
  重新 `configure()`，多轮软件触发每轮都要重新 `start()`。
- **DMA 源/目的地址不能是 DMA/FLASH/RAM 控制器自己的寄存器**（手册
  §8.8.6 注），地址自增量由位宽决定（8/16/32 位 → 1/2/4 字节）。
- **RTC 解锁钥匙 0xCA→0x53**：每次写寄存器都要解锁、写完重新上锁；
  停表状态下不要等 `CR1.WAIT`——它不会清零，运行中写才需要等。
- **本板 LSE 不起振**：没焊 32.768k 晶体（或负载电容不匹配），RTC
  例程会自动退回 LSI（`CLK=2`），这是预期行为不是故障。
- **IWDT 超时是近似值**：RC10K 有 ±% 级漂移，`TimeoutMs(1500)` 名义
  1.5 s，实跑可能在 1.45~1.55 s 之间——喂狗间隔要留足余量。
- **WWDT 过早喂狗会复位**：喂狗必须落在窗口内；用 HAL 的 `feed()`
  会提前拿到 `Err(TooEarly)` 挡一道，`feed_forced()` 才真写。
