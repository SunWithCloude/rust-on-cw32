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
