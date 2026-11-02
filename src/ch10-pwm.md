# 第 10 章 定时器与 PWM

这一章你会学到：

- GTIM（通用定时器）的寄存器模型：PSC/ARR/CCR/CCMR 如何协作生成一路 PWM；
- `GtimPwm` 的构造参数与 `PwmConfig`，以及万分比占空比（`duty_bp`）的语义；
- 运行中改占空比为什么"下一周期才生效"——影子寄存器与 ARPE；
- `wait_update().await` 如何把"PWM 周期边界"变成一个异步节拍。

## GTIM 寄存器模型

CW32L012 有 BTIM1–3（基本定时器）、GTIM1–4（通用定时器）和 ATIM（高级
定时器）三档。本书进阶篇只展开 GTIM——它最能代表"定时器 = 计数器 +
比较器"这套通用模型，ATIM 的互补 PWM 与死区留给需要电机控制的读者按
手册 §17 自学。GTIM 的寄存器面在用户手册 §16。

一路 PWM 由四个寄存器决定（§16.10）：

- **PSC**（预分频）：定时器时钟 PCLK 先除以 `PSC+1` 得到计数时钟
  CK_CNT；
- **ARR**（自动重装载）：计数器 CNT 在 `0..=ARR` 之间滚动，决定 PWM
  周期：`f_pwm = PCLK / ((PSC+1) × (ARR+1))`；
- **CCR**（捕获/比较）：CNT 每个数都与 CCR 比较；
- **CCMR/CCER**（捕获比较模式/使能）：比较结果如何驱动输出引脚。

`embassy-cw32` 的 `GtimPwm` 固定配成**边沿对齐 PWM 模式 1、高有效**
（CNT < CCR 时输出有效电平，CCMRx 的 OCyM 位段，§16.10），这套寄存器配方与
阻塞版 `cw32l012-hal` 的 `gtim.rs` 逐行同源——后者已经上板点过灯（hal 仓
TESTING.md：GTIM1 1kHz，ARR=3999/PSC=0，CNT 铺满 0..3999）。

`GtimPwm::new` 内部自动完成三件事：开 `APBEN1.GTIMx` 时钟门控（§4，
GTIM1..4 = bit6/7/11/12，带 KEY 写保护）、把引脚配成 AF 复用输出、选
PSC/ARR 并启动计数。选档策略是 **PSC 取最小值使 ARR 尽量大**——ARR 越
大，占空比的量化步距越细。比如 4 MHz PCLK 生成 1 kHz：4000 个计数档
≤ 65536，于是 PSC=0、ARR=3999。

## 占空比：万分比满量程

`PwmConfig::duty_bp` 和 `set_duty_bp(duty_bp)` 都用**万分比**表示占空比：
0 = 常低、10000 = 常高、5000 = 50%。满量程固定 10000 而不是"ARR+1"，是
因为 `embedded-hal` 的 `SetDutyCycle` trait 用 u16 返回满量程，而 ARR+1
最大可到 65536 放不下——固定万分比后 `GtimPwm` 同时实现了该 trait，接
`embedded-hal` 生态的呼吸灯/舵机驱动不用改一行。

## ARPE：为什么改占空比"下一周期生效"

写 CCR 不会立刻改变输出波形——CCMR 里开了 **OCyPE 预装载**（CCMRx 的
OCyPE 位，§16.10），
你写的值先进影子寄存器，等下一个更新事件（CNT 回零）才整体换装。这保证
一次占空比修改是**原子**的：波形上不会出现半个错误周期。CR1.ARPE 同理
作用于 ARR（§16.3.1.4）。副作用是：`set_duty_bp` 返回时波形还没变，要
"每周期变一步"的呼吸效果，得等更新事件——这正是 `wait_update` 的用途。

## wait_update：把周期边界变成异步节拍

`GtimPwm::wait_update().await` 等一次真实的更新事件（ISR.UIF），
返回时刻就是一个 PWM 周期边界。实现上是 embassy 标准套路：调用时先清掉
残留 UIF（`GtimPwm::new` 配置时写 EGR.UG 会连带置一次 UIF，§16.10.5），
等待期间才开 DIER.UIE（§16.10.4），中断唤醒后立即压掉使能位（防中断
风暴，纪律与 UART 驱动一致）。它依赖 GTIM1 中断（IRQ16）通过
`bind_interrupts!` 绑定到 `timer::Gtim1Irq`——**只用 PWM、不调
`wait_update` 则完全不需要绑定任何中断**。

GTIM3/GTIM4 共享一条中断线（IRQ18 `GTIM34`），驱动提供 `Gtim34Irq` 标记
成对派发；本例只用 GTIM1，不涉及。

## 例程

`examples/embassy/examples/ch10_pwm.rs`：GTIM1_CH1 输出到 PB04（AF6，
手册表 9-2），1 kHz、占空比 10%→90%→10% 扫动，每个 PWM 周期走一步；
PC13 每 500 个更新事件翻转一次（1 kHz 节拍下 = 1 Hz 心跳）。观察量落
SRAM 静态量，可用 probe-rs 读（地址以 `nm` 输出为准）。

```rust
//! 第 10 章例程：GTIM1 单通道 PWM（PB04，1kHz，占空比 10%→90% 扫动）。
//!
//! 每个 PWM 周期用 `wait_update().await` 节拍（更新中断 UIF + waker），
//! PC13 每 500 个更新事件翻转一次（1kHz 节拍下 = 1Hz 心跳）。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch10_pwm -p cw32-book-embassy
//! 状态：仅编译未上板（embassy 版 timer_pwm 同状态）；寄存器配方与阻塞版
//!       gtim_pwm 同源且已实机验证（ARR=3999/PSC=0、CNT 铺满，见 hal
//!       TESTING.md）。首次上板核对点见第 10 章正文。

#![no_std]
#![no_main]

use embassy_executor::main;

use cw32_book_embassy as _; // 继承 lib.rs 的 panic handler（panic-probe 同款机制）
use embassy_cw32::{
    Config, Peripherals, SYSCTRL_KEY,
    gpio::{Level, Output, Pin},
    timer::{GtimPwm, PwmChannel, PwmConfig},
};

// wait_update 需要 GTIM1 更新中断（IRQ16）；纯 PWM 用法无需绑定任何中断
embassy_cw32::bind_interrupts!(struct Irqs {
    GTIM1 => embassy_cw32::timer::Gtim1Irq;
});

/// (ARR<<16 | PSC) 配置回读：1kHz@PCLK 4MHz 预期 0x0F9F_0000
#[used]
#[unsafe(no_mangle)]
pub static mut PERIOD: u32 = 0;
/// 占空比读回（万分比，由 CCR/ARR 反算）
#[used]
#[unsafe(no_mangle)]
pub static mut DUTY_RB: u16 = 0;
/// wait_update 完成次数（预期 ≈1000/s）
#[used]
#[unsafe(no_mangle)]
pub static mut UPDATES: u32 = 0;
/// 错误码：0=无、1=构造失败（频率超范围/占空比越界，本例常量不会触发）
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // GPIOB（PB04 PWM 脚）+ GPIOC（PC13 LED）同写一个 AHBEN：L012 带
    // KEY 写保护、整字写，分开两次写会互相顶掉使能位（B=5/C=6）
    p.sysctrl
        .ahben
        .write(SYSCTRL_KEY | (1 << 5) | (1 << 6));
    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    // PWM：PB04 = GTIM1_CH1（AF6，手册表 9-2）；1kHz、初始占空比 10%
    let mut pwm = match GtimPwm::new(
        p.gtim1,
        Pin::new(p.gpiob, 4),
        6,
        PwmConfig {
            channel: PwmChannel::Ch1,
            freq_hz: 1_000,
            duty_bp: 1_000, // 万分比：10000 = 100%
        },
    ) {
        Ok(t) => t,
        Err(_) => {
            unsafe { ERR = 1 };
            loop {
                core::hint::spin_loop();
            }
        }
    };

    let (arr, psc) = pwm.period_config();
    unsafe { PERIOD = (arr as u32) << 16 | psc as u32 };

    // 占空比 10%→90%→10% 循环扫动（每步 5%，每个 PWM 周期走一步）
    let mut duty: u16 = 1_000;
    let mut dir_up = true;
    loop {
        // 万分比直接控（ARPE 预装载 → 下一更新周期生效）
        pwm.set_duty_bp(duty).ok();
        // 等一个真实 PWM 周期（GTIM1 更新中断唤醒；返回时刻 = 周期边界）
        pwm.wait_update().await;
        unsafe {
            DUTY_RB = pwm.duty_bp();
            UPDATES += 1;
            if UPDATES % 500 == 0 {
                led.toggle(); // 1kHz 节拍下 500 次 = 1Hz 心跳
            }
        }
        if dir_up {
            duty += 500;
            if duty >= 9_000 {
                dir_up = false;
            }
        } else {
            duty = duty.saturating_sub(500);
            if duty <= 1_000 {
                dir_up = true;
            }
        }
    }
}
```

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch10_pwm -p cw32-book-embassy
```

## 板上验证

**状态：仅编译未上板。** 本例程与 embassy-cw32 仓的 `timer_pwm` 示例
同源，该示例在其 CHANGELOG 0.4.0 的 timer 条目下属于"寄存器配方与阻塞
版同源"的新增项，embassy 版尚未上板。**异步路径（`wait_update` 的 UIF
中断 + waker 唤醒）是首次上板要核对的增量**——阻塞版只验证过纯寄存器
配方的 PWM 输出。

首次上板核对点：

- `PERIOD` 应为 `0x0F9F_0000`（ARR=3999、PSC=0，1 kHz @ PCLK 4MHz）；
- `UPDATES` 以 ≈1000/s 线性增长（证明 `wait_update` 真在等更新中断，
  而不是空转）；PC13 呈 1 Hz 心跳；
- `DUTY_RB`（由 CCR/ARR 反算）应逐步跟随设定值；
- 示波器/逻辑分析仪接 PB04：1 kHz 方波，占空比呈 10%↔90% 呼吸扫动。
  若手头没有 GTIM1_CH1 引出的板子，可把通道/引脚/AF 换成表 9-2 里你
  板上可用的一组（如 GTIM1_CH3 @ PA03 = AF6）。

## 常见坑

- **AHBEN 整字写顶掉别的使能位**：L012 的 SYSCTRL 门控带 KEY 写保护，
  `ahben.write(KEY | bits)` 是整字写——例程里 GPIOB 和 GPIOC 必须一次
  写完，分两次写会让先写的那位被顶掉（KEY 语义见 §4.7）。
- **`wait_update` 未绑定中断 = 首个周期挂死**：NVIC 线落到弱默认向量，
  UIF 置位后无人处理。要么 `bind_interrupts!` 绑 `Gtim1Irq`，要么不用
  `wait_update`。
- **占空比写 10000 才是常高**：`set_duty_bp(10_000)` 内部把 CCR 配到
  ARR+1，CNT 永远小于 CCR，输出恒为有效电平；写 9999 则每个周期还有
  一个计数档的低电平毛刺。
- **引脚与通道必须对上**：`PwmChannel::Ch1` 配 PB04 时 AF 必须是 6
  （表 9-2）；AF 配错时 GTIM 在正常计数、引脚上却看不到波形。
- **频率可实现的边界**：`f > PCLK/2` 或 `f < PCLK/65536²` 时构造返回
  `PwmError::OutOfRange`（PSC/ARR 都救不回来）；复位默认 4 MHz PCLK
  下，1 kHz 这类常用频率裕量极大。
