# 第 15 章 综合实战

这一章你会学到：

- 怎么把前 14 章的四块积木（GPIO、UART、ADC、PWM）拧成**一个能跑的小项目**
  ——串口小命令台（serial command console）；
- 集成层的三个新问题：时钟门控的组合字、`bind_interrupts!` 的多路绑定、
  构造顺序与 APBEN1 的恢复；
- 任务划分：命令解析任务独占全部外设、心跳任务零外设——"所有权即权限"
  的划分法；
- 跨任务共享状态的最小集合：为什么 thumbv6m 上累加计数要用 critical-section
  而不是原子量；
- 从例程到自己工程的最小目录结构，以及首次上板的逐条核对清单。

## 设计：一个"骨架工程"长什么样

前 14 章是按层教的：PAC 裸寄存器（第一部分）、阻塞 HAL（第二部分）、
embassy 异步（第三部分）、外设进阶（第四部分）。每一章的例程都只演示
一件事。收官章反过来：**功能很普通，集成是真题**——把学过的四块积木
装进一个固件，让它们同时活着、互不拆台。

项目选了"串口小命令台"：一条 USB 转串口线，上电就是一个交互 shell。
命令协议五行：

| 命令 | 作用 | 用到的积木（出处） |
|---|---|---|
| `led on` / `led off` | PC13 亮 / 灭（极性视板卡而定） | `gpio::Output`（第 7 章） |
| `adc` | PA0 单次采样，报原始码 + mV | `adc::Adc` 的 `read` + `vdda_mv`（第 11 章） |
| `pwm <0..10000>` | GTIM1@PB04 占空比运行中调整 | `timer::GtimPwm` 的 `set_duty_bp`（第 10 章） |
| `info` | 心跳计数 + PWM 占空比 + 最近一次 ADC | 双任务 + critical-section |
| 其他 | 复述命令词 + 给命令表（错误路径） | — |

任务就两个，跑在同一个执行器上（第 7 章）：

```text
USB-TTL ──UART1──► console 任务（独占 rx/tx/adc/pwm/led）
                     │   rx.read → 拼行 → exec() → reply
                     ▼
                     ├─► Output(PC13)        （led 命令）
                     ├─► Adc(PA0)            （adc 命令）
                     └─► GtimPwm(PB04)       （pwm 命令）

heartbeat 任务（零外设）──1s──► TICKS +1 ──critical-section──► info 读
```

`console` 任务拿到**全部**外设的所有权，`heartbeat` 任务一个外设都不碰
——这个划分是本章的核心手法，拼装点 3 细讲。四路外设配方全部来自前文，
例程里真正**新**的只有集成层的三件事，下面逐个说。

## 拼装点 1：时钟门控——三个端口一个字，三路外设一个字

第 7/8 章踩过一次：AHBEN 是键控整字写（高 16 位必须 KEY = 0x5A5A），分两次
写会互相顶掉。本例程要同时用三个端口——GPIOA（PA0/PA02/PA03）、
GPIOB（PB04）、GPIOC（PC13），对应 AHBEN 的 bit4/5/6（手册 §4.7），
只能一次写齐：

```rust,ignore
p.sysctrl.ahben.write(SYSCTRL_KEY | (1 << 4) | (1 << 5) | (1 << 6));
```

APBEN1 上的问题更隐蔽，它藏在构造顺序里。三个外设驱动**各自**开自己的
时钟，而且姿势不同（embassy-cw32 源码）：

- `BufferedUart::new_uart1`：APBEN1 **整字写** `KEY | bit3`
  （`src/uart.rs`，UART1 = bit3）；
- `Adc::new_adc1`：APBEN1 **整字写** `KEY | APBEN1_ADC`
  （`src/adc.rs`；PAC 里 `APBEN1_ADC = 1 << 0`）；
- `GtimPwm::new`：APBEN1 **读改写**追加 GTIM1 位（`src/timer.rs`，
  GTIM1 = bit6）。

连着调，前两个整字写互相顶掉：ADC 构造完，UART1 的时钟位已经丢了。
单外设例程从没暴露这个问题——每个驱动的构造函数都默认"我是唯一在配
时钟的外设"。多外设工程有两个选择：给上游驱动提 patch 改成读改写
（超出本章范围，是给生态仓的合理贡献），或像本例程一样**构造完统一补
一次组合字**：

```rust,ignore
p.sysctrl.apben1.write(SYSCTRL_KEY | (1 << 0) | (1 << 3) | (1 << 6));
```

注意这行补的是**时钟**，不是配置：三个构造函数执行时各自时钟都开着，
寄存器配置已经写进外设了；末尾恢复时钟后配置原样生效。顺序别倒过来
——先关门再配置，写进的是没人听得见的房间。

## 拼装点 2：bind_interrupts! 的多路绑定

本例程两路中断要绑：UART1（IRQ27，排空 RX 管道）和 ADC1（IRQ12，转换
完成唤醒）。`bind_interrupts!` 一个 struct 绑多路，分号分隔即可：

```rust,ignore
embassy_cw32::bind_interrupts!(struct Irqs {
    UART1 => uart::Uart1Irq;
    ADC1 => embassy_cw32::adc::Adc1Irq;
});
```

宏展开出两个强符号向量函数（覆盖 PAC `device.x` 的弱默认），并给 `Irqs`
实现两个 `Binding<...>`——构造函数的编译期校验照常起作用（第 9 章）。

GTIM1 刻意**不绑**：只有 `wait_update()` 需要 GTIM1 更新中断（IRQ16），
本例程纯 `set_duty_bp` 改占空比，不产生中断。多绑也能编译（向量函数
存在、没人开 NVIC 线，无副作用，第 9 章），但"绑了不用"是噪声——
绑中断清单应该是"谁在等中断"的准确账本。

## 拼装点 3：任务划分——所有权即权限

`console` 任务的签名就是权限清单：rx、tx、adc、pwm、led 五个参数，
全部经 `spawner.spawn(...)` 把所有权**移入**任务（第 7 章：spawn 参数
打包进 `SpawnToken`，静态存储一份，重复 spawn 报 `SpawnError::Busy`）。
命令执行时五个外设全是 `&mut` 独占访问——单任务持有，无竞争，
`exec` 不需要任何锁。

`heartbeat` 任务是对照面：零外设。它只碰 `TICKS` 计数和 RTT 日志，
证明"执行器活着"这件事不需要占用任何硬件外设——第 8 章的心跳 LED
在命令台里反而不能用了（PC13 的所有权在 console 手里，`led on/off`
要它听命令的，不能被心跳抢着翻）。

`main` 的最后两行 spawn 完就退场：`#[main]` 函数体结束即进入执行器的
调度循环（第 7 章），两个子任务继续被轮询。划分原则一句话：
**能被所有权解决的共享，就不要用锁**。把外设全部划给一个任务，剩下
真正需要同步的状态才浮出水面——这里只剩一个 `TICKS`。

## 拼装点 4：唯一的跨任务共享状态

`TICKS` 是 heartbeat 独写、console 经 `info` 读的 u32 计数。直觉答案是
原子量，但 thumbv6m 上行不通：

- **硬件层**：Cortex-M0 没有 LDREX/STREX，原子读改写指令在 ARMv6-M 上
  不存在（它们是 ARMv7-M 才引入的）；
- **库层**：本仓工具链（rustc 1.97 实测）上，该目标的
  `core::sync::atomic::AtomicU32` 只有 `load`/`store`——`fetch_add`/
  `swap` 这类方法直接 E0599，不存在。

所以累加用 critical-section 守护——第 14 章静态记录区同款配方
（`Mutex<RefCell<u32>>` + `critical_section::with`）。临界区实现由
embassy-cw32 默认 feature 提供（PRIMASK 关中断；第 3 章三件套里的
defmt-rtt 用的就是同一个实现——一个固件只能有一个实现，第 7 章讲过
混用两个实现的链接期冲突）。

再往上的升级路径是 embassy-sync：状态变成多字段用
`Mutex<Cell<T>>`（免临界区的异步 Mutex），事件通知用 `Signal`。本例
一个计数用不上——**共享状态从最小集合起步，长了再换工具**，"下一步"
一节会给一个真正需要 `Signal` 的改造练习。

## 例程

`examples/embassy/examples/ch15_console.rs` 全文（构建命令见清单后）：

```rust
//! 第 15 章例程：串口小命令台（收官——把前 14 章的积木拧成一个项目）。
//!
//! UART1（PA02=TXD / PA03=RXD，AF1，115200-8N1，BufferedUart）收行命令：
//! `led on|off`（PC13，亮灭极性视板卡而定）、`adc`（PA0 单次采样 + VDDA 折算
//! mV）、`pwm <0..10000>`（GTIM1@PB04 占空比运行中调整）、`info`（运行
//! 状态）；未知命令回错误提示。命令行任务独占全部外设，心跳任务只碰
//! critical-section 计数 + RTT 日志——双任务跑在同一个执行器上。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch15_console -p cw32-book-embassy
//! 状态：仅编译未上板。四路配方逐一来自前文：BufferedUart（第 8 章，同源
//!       已验证）、ADC（第 11 章，寄存器配方与阻塞版同源）、PWM（第 10 章，
//!       同源）；集成层（APBEN1 组合字、行协议、任务划分）为本章新增，
//!       首次上板核对清单见第 15 章正文。

#![no_std]
#![no_main]

use core::{cell::RefCell, fmt::Write as _};

use critical_section::Mutex;
use embassy_executor::{main, task};
use embassy_time::Timer;

use cw32_book_embassy as _; // 继承 lib.rs 的 panic handler（panic-probe 同款机制）
use defmt_rtt as _; // RTT 日志通道（第 3 章三件套的 defmt-rtt）
use embassy_cw32::{
    Config, Peripherals, SYSCTRL_KEY,
    adc::{Adc, AdcChannel, AdcClk, AdcError, SampleCycles},
    gpio::{Level, Output, Pin},
    timer::{GtimPwm, PwmChannel, PwmConfig},
    uart::{self, BufferedUart, BufferedUartRx, BufferedUartTx},
};

// 两路中断一次绑齐：UART1（IRQ27，排空 RX 管道）+ ADC1（IRQ12，转换完成
// 唤醒）。GTIM1 不绑——纯 set_duty 用法不碰 wait_update，不产生中断。
embassy_cw32::bind_interrupts!(struct Irqs {
    UART1 => uart::Uart1Irq;
    ADC1 => embassy_cw32::adc::Adc1Irq;
});

/// 心跳打点计数：heartbeat 任务独写，console 任务经 info 命令读——本例
/// 唯一的跨任务共享状态。thumbv6m 没有原子读改写（Cortex-M0 无
/// LDREX/STREX，core::sync::atomic 在该目标没有 fetch_add/swap 这类
/// 方法），累加用 critical-section 守护（第 14 章静态记录区同款配方）。
static TICKS: Mutex<RefCell<u32>> = Mutex::new(RefCell::new(0));

/// 行缓冲上限（含参数）。超长行报错丢弃，防一条命令灌满输入。
const MAX_LINE: usize = 31;

/// 栈上格式化缓冲：core::fmt::Write 适配（无 alloc 环境拼数字用）。
struct Fmt<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl core::fmt::Write for Fmt<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let rest = &mut self.buf[self.len..];
        let n = s.len().min(rest.len());
        rest[..n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// 发一串字节并等发完（flush 等 TC：最后一个字节真正移位完毕）
async fn reply(tx: &mut BufferedUartTx, msg: &[u8]) {
    tx.write(msg).await;
    tx.flush().await;
}

/// 格式化一条回复并发送：栈缓冲 + format_args! 拼数字（缓冲不够长则截断）
async fn reply_fmt(tx: &mut BufferedUartTx, out: &mut [u8], args: core::fmt::Arguments<'_>) {
    let mut f = Fmt { buf: out, len: 0 };
    let _ = f.write_fmt(args);
    reply(tx, &f.buf[..f.len]).await;
}

/// AdcError → 简短标识（错误路径要能看懂，不能只吞掉）
fn adc_err(e: AdcError) -> &'static str {
    match e {
        AdcError::Timeout => "timeout",
        AdcError::BadCalibration => "bad calibration",
        AdcError::UnsupportedChannel => "unsupported channel",
        AdcError::SensorDead => "sensor dead",
    }
}

/// 解析并执行一行。全部外设经 &mut 独占访问——单任务持有，无竞争。
async fn exec(
    tx: &mut BufferedUartTx,
    led: &mut Output<embassy_cw32::Gpio>,
    adc: &mut Adc,
    pwm: &mut GtimPwm,
    last: &mut [u32; 2], // [0]=VDDA mV，[1]=PA0 原始码（info 命令展示）
    line: &str,
) {
    // 第一个空格切分：cmd + 参数（参数两侧空白容忍）
    let (cmd, arg) = match line.split_once(' ') {
        Some((c, a)) => (c, a.trim()),
        None => (line, ""),
    };
    match cmd {
        "led" => match arg {
            "on" => {
                led.set_low(); // 极性视板卡而定（第 4 章）：首板上板时记录
                reply(tx, b"led: on\r\n").await;
            }
            "off" => {
                led.set_high();
                reply(tx, b"led: off\r\n").await;
            }
            _ => reply(tx, b"usage: led on|off\r\n").await,
        },
        "adc" => {
            // VDDA 先行：内部 1.2V 基准 + 出厂校准（第 11 章配方），作换算分母
            let mv = match adc.vdda_mv().await {
                Ok(mv) => mv,
                Err(e) => {
                    let mut out = [0u8; 48];
                    reply_fmt(tx, &mut out, format_args!("adc: VDDA error ({})\r\n", adc_err(e)))
                        .await;
                    return;
                }
            };
            // PA0 单次转换（外部通道 IN0；悬空读数不定，接分压后随输入变化）
            match adc.read(AdcChannel::In(0), SampleCycles::S24).await {
                Ok(raw) => {
                    last[0] = mv;
                    last[1] = raw as u32;
                    // 12 位满量程换算：raw/4095 × VDDA
                    let pin_mv = (raw as u32 * mv) / 4095;
                    let mut out = [0u8; 72];
                    reply_fmt(
                        tx,
                        &mut out,
                        format_args!("adc: PA0 raw={} ~{} mV (VDDA {} mV)\r\n", raw, pin_mv, mv),
                    )
                    .await;
                }
                Err(e) => {
                    let mut out = [0u8; 48];
                    reply_fmt(tx, &mut out, format_args!("adc: PA0 error ({})\r\n", adc_err(e)))
                        .await;
                }
            }
        }
        "pwm" => match arg.parse::<u16>() {
            Ok(d) => match pwm.set_duty_bp(d) {
                Ok(()) => {
                    let rb = pwm.duty_bp(); // CCR/ARR 反算读回（ARPE 预装载）
                    let mut out = [0u8; 64];
                    reply_fmt(
                        tx,
                        &mut out,
                        format_args!("pwm: duty={}/10000 (readback {})\r\n", d, rb),
                    )
                    .await;
                }
                Err(_) => reply(tx, b"pwm: duty must be 0..=10000\r\n").await,
            },
            Err(_) => reply(tx, b"usage: pwm <0..10000>\r\n").await,
        },
        "info" => {
            let ticks = critical_section::with(|cs| *TICKS.borrow_ref(cs));
            let mut out = [0u8; 96];
            reply_fmt(
                tx,
                &mut out,
                format_args!(
                    "info: ticks={} | pwm={}/10000 | last adc: raw={} vdda={} mV\r\n",
                    ticks,
                    pwm.duty_bp(),
                    last[1],
                    last[0]
                ),
            )
            .await;
        }
        _ => {
            // 未知命令：复述收到的词，再给命令表——错误路径也要友好
            reply(tx, b"unknown command: ").await;
            reply(tx, cmd.as_bytes()).await;
            reply(tx, b"\r\ntry: led on|off | adc | pwm <0..10000> | info\r\n").await;
        }
    }
}

/// 命令行任务：读字节流 → 拼行 → 分发执行。rx/tx/adc/pwm/led 全部由
/// spawn 转入本任务——所有权即权限。
#[task]
async fn console(
    mut rx: BufferedUartRx,
    mut tx: BufferedUartTx,
    mut adc: Adc,
    mut pwm: GtimPwm,
    mut led: Output<embassy_cw32::Gpio>,
) {
    reply(&mut tx, b"\r\n== CW32L012 serial console (Rust on CW32 ch15) ==\r\n").await;
    reply(&mut tx, b"try: led on|off | adc | pwm <0..10000> | info\r\n").await;

    let mut chunk = [0u8; 32];
    let mut line = [0u8; MAX_LINE];
    let mut len = 0usize;
    let mut overflow = false;
    let mut last = [0u32; 2];

    loop {
        let n = rx.read(&mut chunk).await; // 挂起直到管道里至少 1 字节
        tx.write(&chunk[..n]).await; // 整段回显（终端不本地回显时靠这里）
        for &b in &chunk[..n] {
            if b == b'\r' || b == b'\n' {
                // 行结束（终端多发 CR，少数发 LF；CRLF 时 LF 落在 len==0
                // 上被自然忽略）
                if len > 0 || overflow {
                    tx.write(b"\r\n").await;
                    if overflow {
                        reply(&mut tx, b"error: line too long\r\n").await;
                    } else {
                        let s = core::str::from_utf8(&line[..len]).unwrap_or("");
                        exec(&mut tx, &mut led, &mut adc, &mut pwm, &mut last, s).await;
                    }
                    len = 0;
                    overflow = false;
                }
            } else if len < MAX_LINE {
                line[len] = b;
                len += 1;
            } else {
                overflow = true; // 超长：静默吞字节，行尾统一报错
            }
        }
    }
}

/// 心跳任务：1s 打点。不占任何外设——TICKS 计数（console 的 info 命令
/// 读它）+ defmt RTT 日志（probe-rs attach 时可见）。
#[task]
async fn heartbeat() {
    loop {
        Timer::after_millis(1000).await;
        critical_section::with(|cs| *TICKS.borrow_ref_mut(cs) += 1);
        defmt::info!("heartbeat tick");
    }
}

#[main]
async fn main(spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // 三端口一次写齐：AHBEN 是键控整字写（高 16 位必须 0x5A5A），分次写
    // 会互相顶掉（第 10/11 章同款处理）。GPIOA(bit4，PA0/PA02/PA03)、
    // GPIOB(bit5，PB04)、GPIOC(bit6，PC13)
    p.sysctrl.ahben.write(SYSCTRL_KEY | (1 << 4) | (1 << 5) | (1 << 6));

    // LED：复位后 High（亮灭极性视板卡而定，第 4 章）
    let led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    // UART1（第 8 章配方）：构造里 APBEN1 整字写 KEY|UART1
    let u = BufferedUart::new_uart1(
        p.uart1,
        Pin::new(p.gpioa, 2),
        Pin::new(p.gpioa, 3),
        1, // AF1
        Irqs,
        Default::default(), // uart::Config：115200
    );
    let (mut tx, rx) = u.split();

    // ADC1（第 11 章配方）：构造里 APBEN1 又一次整字写 KEY|ADC——刚才的
    // UART1 位被顶掉（末尾统一恢复，见下）。PA0 → 模拟态（IN0）
    let adc = Adc::new_adc1(p.adc1, AdcClk::Div2, Irqs);
    let _pa0 = Pin::new(p.gpioa, 0).into_analog();

    // GTIM1 PWM（第 10 章配方）：构造里 APBEN1 是读改写，追加 GTIM1 位
    let pwm = match GtimPwm::new(
        p.gtim1,
        Pin::new(p.gpiob, 4),
        6, // GTIM1_CH1@PB04 = AF6（手册表 9-2）
        PwmConfig {
            channel: PwmChannel::Ch1,
            freq_hz: 1_000,
            duty_bp: 0, // 上电 0%：等 `pwm` 命令调整
        },
    ) {
        Ok(t) => t,
        Err(_) => {
            // 本例常量不会触发（1kHz@4MHz 在范围内）；真失败就报错停机
            reply(&mut tx, b"fatal: pwm init failed\r\n").await;
            loop {
                core::hint::spin_loop();
            }
        }
    };

    // APBEN1 收尾：三个构造函数各写各的（两个整字写互顶 + 一个读改写），
    // 末尾一次组合字对齐三个使能位（ADC=bit0、UART1=bit3、GTIM1=bit6，
    // 手册 §4）。恢复的是时钟——各外设的寄存器配置在构造时已写入。
    p.sysctrl.apben1.write(SYSCTRL_KEY | (1 << 0) | (1 << 3) | (1 << 6));

    // 双任务：console 独占全部外设（所有权经 spawn 转入），heartbeat 只
    // 碰 TICKS + RTT。main 到这儿就退场——#[main] 展开的 executor run()
    // 循环永不返回，两个子任务继续被调度。
    spawner.spawn(console(rx, tx, adc, pwm, led).unwrap());
    spawner.spawn(heartbeat().unwrap());
}
```

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch15_console -p cw32-book-embassy
```

几个值得停留的细节：

**行协议的拼装**。`rx.read(&mut chunk)` 从管道取一段（挂起直到至少
1 字节，第 8 章），整段回显后再逐字节拼行：`\r`/`\n` 算行尾（终端
多发 CR，少数发 LF；CRLF 时 LF 落在空行上被自然忽略），普通字节追加
进 `line`。超长行（>31 字节）静默吞掉、行尾统一报
`error: line too long`——有上限没报错更危险，截断会把 `led onxxxx`
变成另一条命令。

**无 alloc 环境拼数字**。回复里的数字靠 `reply_fmt`：栈上字节数组 +
`Fmt`（实现 `core::fmt::Write`）+ `format_args!`，缓冲不够长就截断。
没有 alloc 就没有 `String`，`write!`/`format_args!` 这套机器要自己接一个
"落到字节数组"的 `Write`——十几行，一次写好处处用。

**PWM 运行中调整**。`pwm 5000` → `set_duty_bp(5000)` 写 CCR（ARPE
预装载，下一更新周期生效，第 10 章），`duty_bp()` 从 CCR/ARR 反算
读回——回复里的 readback 就是"寄存器真的变了"的证据。

**错误路径都开口**。未知命令复述词 + 命令表；参数不合法回 usage；
ADC 每个错误变体经 `adc_err` 翻译成人话（四个变体含 `SensorDead`，
第 11 章的温度传感器个体差异）；PWM 构造失败（本例常量不会触发）报
fatal 后停机。错误不吞、不猜、不复位。

## 从例程到工程：目录结构建议

把例程变成自己的工程，最小改动三步：新建工程目录、抄依赖清单、搬
`main.rs`。

```text
my-console/
├── .cargo/config.toml   # target = thumbv6m-none-eabi；probe-rs runner（附录 D）
├── Cargo.toml           # 依赖清单照抄 examples/embassy/Cargo.toml 的 [dependencies]
└── src/
    ├── main.rs          # ch15_console.rs 整体就是起点
    ├── protocol.rs      # exec() 与命令表——命令多起来第一个拆它
    └── codec.rs         # Fmt / reply / reply_fmt——拼行与回复
```

两条原则：**先别拆**——单文件 300 行以内时拆分是负资产，等命令超过
一屏再拆 `protocol.rs`；**依赖别增**——例程的 dev-dependencies
（embassy-executor/embassy-time/defmt 三件套/critical-section）就是
embassy 路线的全部家当，加依赖前先查附录 B 有没有现成的。

烧录与观测：

```bash
probe-rs run --chip CW32L012x8 target/thumbv6m-none-eabi/release/examples/ch15_console
```

`probe-rs run` 烧录 + 复位 + 自动附加 RTT（附录 D），heartbeat 任务的
`defmt::info!` 打点直接打到终端。

## 板上验证

**状态：仅编译未上板。** 四路配方各自的单外设用法在前文各有验证记录
（BufferedUart 同源已验证、ADC/PWM 寄存器配方与阻塞版同源），但**这个
组合形态没有以整体烧过板**：集成层（APBEN1 组合字、双中断绑定、双任务、
行协议）是本章新增代码路径，没有实测记录。首次上板逐条核对：

- **硬件准备**：USB 转串口适配器交叉接线（适配器 RXD → 板 PA02、适配器
  TXD → 板 PA03、GND 共地，第 5 章同款）；PA0 接电位器分压（可选，悬空
  读数不定属正常）；PB04 接示波器或逻辑分析仪（可选）；probe-rs 探针
  接 SWD 看 RTT。
- **核对清单**（上电后按序执行）：
  1. 复位后终端出现横幅与 `try:` 提示——BufferedUart 发送路径 + flush
     （TC 等"最后一个字节移位完毕"）连通；
  2. `led on` / `led off` → PC13 亮灭且回复正确（顺带记录本板极性：
     亮对应 `set_low` 还是 `set_high`，第 4 章口径"视板卡而定"）；
  3. `adc` → VDDA 落在 3200~3300 mV（hal 仓 TESTING.md 实测 3282 mV，
     第 11 章）；PA0 接分压后 raw 与 mV 随输入变化；
  4. `pwm 5000` → PB04 出现 1kHz、约 50% 占空比；`pwm 0` / `pwm 10000`
     → 恒低 / 恒高；`pwm 10001` → usage 提示；
  5. `info` → `ticks` 每秒 +1，与 RTT 终端的 `heartbeat tick` 同步
     （双任务都在跑的证据）；
  6. `hello` → `unknown command: hello` + 命令表；
  7. 粘贴一行 >31 字符 → `error: line too long`；
  8. 完全无横幅：先查 TX/RX 交叉与共地、波特率 115200-8N1（第 5 章
     同款核对），再按附录 C 排查。

## 常见坑

- **忘了 APBEN1 收尾组合字**：横幅一条都收不到——UART1 的时钟位被
  ADC 构造的整字写顶掉了，TX 管道有数据但没有时钟把它移出去。这是本
  例程最反直觉的一处，1 秒钟修好（补一行写），找起来要花一下午。
- **把 embassy-sync 当默认武器**：一个 u32 计数上异步 Mutex 是杀鸡用
  牛刀；critical-section 配 `RefCell` 更短且已有实现。反过来，状态多
  字段或要"跨任务等事件"时，别手搓标志位轮询，直接上 embassy-sync。
- **行缓冲无上限**：`line` 数组写穿（踩内存或 panic）；或反之有上限
  无报错，超长命令被静默截断成另一条命令。
- **改频之后全部重算**：本例程全程复位默认 4 MHz；一旦提频，UART 波特
  率与 1 kHz PWM 的 ARR/PSC 都是驱动按当前 PCLK 推导的，构造参数要重配
  ——提频本身不在本书路线里，改时钟前先重读第 1/2 章的复位默认口径。
- **ADC 命令放进紧循环**：`vdda_mv()` 内部等 BGR 稳定期，一次 `adc`
  命令毫秒级；命令台场景无感，写进 1 ms 轮询就是事故。

## 下一步

骨架工程到手，三个方向的练习按难度递增：

1. **"呼吸灯"改造**：把 `pwm` 划给第三个 `breath` 任务，console 收到
   `breathe on/off` 时经 embassy-sync 的 `Signal` 通知它——你会在
   所有权模型上第一次**真正需要** embassy-sync（拼装点 3/4 留白的地
   方），顺带体会"加一个共享外设，同步结构跟着长"的工程感觉。
2. **ADC 序列 + DMA**：`adc` 命令升级为 `read_sequence` 双通道
   （第 11 章），再用 `read_sequence_dma` 减负（第 12 章）。
3. **RTC 定时命令**：到点自动执行一条预设命令（第 13 章）——命令台
   从"人敲的"长成"机器也会用的"。

移植与生态两问（换家族要动哪些行、哪些 crate 能省事）分别在附录 A 与
附录 B。这本书到此收官——欢迎把你的命令台接上传感器，让 CW32 替你
值班。
