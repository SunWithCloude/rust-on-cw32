# 第 11 章 ADC 与 DAC

这一章你会学到：

- L012 双 ADC 实例（ADC1/ADC2）的通道模型与采样序列（§25）；
- 内部通道怎么用：1.2V 基准反推 VDDA、温度传感器，以及它们的出厂校准；
- `read` / `read_sequence` / `read_sequence_dma` 三条读取路径的区别；
- DAC 双通道直写与 DMA 波形输出（`write_dma`）的"请求搬运"模型（§26）。

## ADC：双实例与通道映射

L012 有两个 12 位 SAR ADC（§25）。**ADC1 覆盖全部通道，ADC2 只有外部
通道**，两者的外部引脚映射还不同（表 25-4）：

- ADC1：IN0..7 = PA0..PA7、IN8 = PB0、IN9 = PB1、IN10 = PB10、
  IN11 = PB2；IN12/IN13 是 DAC 输出的内部互联；IN14 = 温度传感器、
  IN15 = 1.2V 基准（这两个内部通道仅 ADC1 有）；
- ADC2：IN0 = PA05 起的另一套映射，且无内部通道。

外部引脚必须先转模拟模式（`Pin::into_analog()`，置 `GPIOx_ANALOG.PINy=1`，
§25 对模拟通道的要求）。

ADC 时钟 ADCCLK = PCLK/2^N（`AdcClk::Div1..Div8`，§25 寄存器 CR 的 CLK
位段），每个通道可独立选采样周期档位（`SampleCycles::S6..S518`，单位
ADCCLK，表 25-2）——高阻抗信号源选长采样，这是 SAR ADC 的通用规则。

## 三条读取路径

`Adc` 驱动（构造用 `Adc::new_adc1(p.adc1, AdcClk::Div2, Irqs)`，绑定
IRQ12 `ADC1`）提供三条异步读取路径：

- **`read(ch, sample)`**——单次转换一个通道，等 EOC（转换完成）中断
  唤醒，返回 0..4095；
- **`read_sequence(&chs, &samples, &mut out)`**——序列扫描：一次启动
  按序转换 2..=8 个通道，等 EOS（整段序列完成）后按通道顺序填 `out`；
- **`read_sequence_dma(&mut dma, &chs, &samples, &mut out)`**——上者的
  DMA 版：序列完成（EOS）时 ADC 发出一个 DMA 请求，DMA 通道把整段
  结果一次搬进缓冲，CPU 不逐通道取数。

三条路径都会自动处理内部通道的前置条件：BGREN（1.2V 基准使能，复位
默认已开）和 TSEN（温度传感器），并异步等待稳定期。

## 内部通道与出厂校准

**VDDA 测量**：读 IN15（1.2V 基准）得到 raw，配合 Flash 信息区 0x0010_07D2
处的 16 位出厂校准值 cal，`VDDA = cal × 4095 / raw`（手册 §25.8 步骤 13）。
`adc.vdda_mv().await` 直接返回毫伏。

**温度测量**：读 IN14（温度传感器），公式 `T = T0×0.5 +
0.095765×(Vref×AdcValue − 4.095×Trim)`（§25.9），T0/Trim 同样取自出厂
校准字。`temperature_mc()` 返回毫摄氏度并自动先测 VDDA。

⚠ **温度传感器在部分硅片上无输出是已知个体差异**：本书作者的一块板
经三轮受控实验（使能顺序、采样档位、POR 前后对比）确认该芯片 TS 模拟
块输出级无输出（0~7 counts），而同路径 BGR/外部通道全部正常——硬件个体
状态，非驱动问题。为此驱动加了失效保护：原始值 <100 counts（正常室温
≈750~1100）时 `temperature_mc*()` 返回 `AdcError::SensorDead` 而不是
算出"-271℃"式的假数据。你在自己的板上读温度前，先看 `read()` 的原始
值是否在正常范围。

## DAC：直写与请求搬运

L012 的 DAC 是双通道 12 位（§26）：CH1 引脚输出 PA4（DAC_OUT1）、CH2
是 PA5（DAC_OUT2）——注意这两个引脚同时是 SPI1 的复用脚，官方开发板
把它们分配给了屏幕；**CH2 在片内直连 ADC1_IN12**，不经过引脚，随时可以
做"写 DAC → ADC 读回"的零接线自检（阻塞版实测最大误差 3 LSB，见 hal
仓 TESTING.md）。

直写模式（复位默认，TENx=0）：`set_ch1/set_ch2/set_both` 写 DHR 后
1 个 dac_pclk 内载入 DOR，输出立即改变——这是板上验证过的路径。

DMA 波形输出（§26.7）是**请求搬运**模型：`CR0.DMAENx=1` 且 DMA 通道配
硬件触发源 = 对应 DAC 通道（表 8-8：`hs::DAC1`/`hs::DAC2`）时，DAC 完成
一次 DOR 加载就自动请求 DMA 把下一个样点搬进 DHR。由此每个样点的时序是
"触发 → 播放当前样点 → 发请求 → DMA 预载下一样点"，**N 个样点需要 N+1
次触发才完整播出**——`write_dma` 收尾时把末样点经直写路径补进 DOR，对
调用方呈现"整段已播完"。供拍源用定时器的 TRGO（表 26-1 选 TSEL），本例
用 GTIM1 配 1 kHz 更新触发。

## 例程 1：ADC 外部通道 + VDDA

`examples/embassy/examples/ch11_adc.rs`：零接线可跑。PA0（IN0）悬空时
读数不定，接电位器/分压后随输入变化；VDDA 走内部基准 + 出厂校准，预期
3200~3300 mV。

```rust
//! 第 11 章例程：ADC1 异步单次转换（外部通道 PA0/IN0 + 内部基准测 VDDA）。
//!
//! 零接线可跑：PA0 悬空读数不定（验证通路本身），VDDA 走 IN15 内部
//! 1.2V 基准 + 出厂校准，预期 ~3200-3300 mV。PC13 每轮翻转。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch11_adc -p cw32-book-embassy
//! 状态：仅编译未上板（embassy 版 adc_async 同状态）；VDDA/内部通道配方
//!       与阻塞版 adc_internal 同源且已实机验证（VDDA=3282mV，见 hal
//!       TESTING.md）。首次上板核对点见第 11 章正文。

#![no_std]
#![no_main]

use embassy_executor::main;
use embassy_time::Timer;

use cw32_book_embassy as _; // 继承 lib.rs 的 panic handler（panic-probe 同款机制）
use embassy_cw32::{
    Config, Peripherals, SYSCTRL_KEY,
    adc::{Adc, AdcChannel, AdcClk, SampleCycles},
    gpio::{Level, Output, Pin},
};

embassy_cw32::bind_interrupts!(struct Irqs {
    ADC1 => embassy_cw32::adc::Adc1Irq;
});

/// 最近一轮结果：[0]=VDDA（mV），[1]=PA0 原始码（0..4095）。
/// 单任务独写 + probe-rs 只读，static mut 可接受。
#[used]
#[unsafe(no_mangle)]
pub static mut RESULTS: [u32; 2] = [0; 2];
/// 错误码：0=无 1=Timeout 2=BadCalibration 3=UnsupportedChannel
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;
/// 完成的轮数（心跳计数）
#[used]
#[unsafe(no_mangle)]
pub static mut SAMPLES: u32 = 0;

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // GPIOA（PA0 模拟脚）+ GPIOC（PC13 LED）同写一个 AHBEN：KEY 整字写，
    // 分两次写会互相顶掉使能位（A=4/C=6）
    p.sysctrl.ahben.write(SYSCTRL_KEY | (1 << 4) | (1 << 6));
    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    let mut adc = Adc::new_adc1(p.adc1, AdcClk::Div2, Irqs);
    // PA0 → 模拟模式（IN0；模拟外设要求 GPIOx_ANALOG.PINy=1）
    let _pa0 = Pin::new(p.gpioa, 0).into_analog();

    loop {
        // VDDA：内部 1.2V 基准通道 + 出厂校准（BGREN 稳定期由驱动处理）
        match adc.vdda_mv().await {
            Ok(mv) => unsafe { RESULTS[0] = mv },
            Err(e) => unsafe {
                ERR = match e {
                    embassy_cw32::adc::AdcError::Timeout => 1,
                    embassy_cw32::adc::AdcError::BadCalibration => 2,
                    _ => 3,
                }
            },
        }
        // PA0 外部通道（悬空时读数不定，接电位器/分压后随输入变化）
        if let Ok(raw) = adc.read(AdcChannel::In(0), SampleCycles::S24).await {
            unsafe { RESULTS[1] = raw as u32 };
        }
        unsafe {
            SAMPLES += 1;
            led.toggle();
        }
        Timer::after_millis(500).await;
    }
}
```

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch11_adc -p cw32-book-embassy
```

## 例程 2：DAC DMA 三角波

`examples/embassy/examples/ch11_dac_wave.rs`：GTIM1 配 1 kHz 更新 +
TRGO（CR2.MMS=010），DAC1 触发源选 `DacTrig::Gtim1Trgo`，`write_dma`
播 32 点三角波（32 ms 一轮）。默认不使能引脚输出（PA4 与 SPI1 引脚
冲突），要看波形需补一行 `dac.enable_pin1()` 并把探头接 PA4；零接线
判据是 `ERR=0` 且 `ROUNDS` 递增、`LAST_DOR` 恒 0（三角波终点 = 末样点
补播生效）。

```rust
//! 第 11 章例程：DAC1 DMA 三角波输出（GTIM1 TRGO 供拍，零接线可跑）。
//!
//! 触发链：GTIM1 配 1kHz 更新 + TRGO（CR2.MMS=update）→ DAC1 触发源
//! TSEL=GTIM1_Trgo → 每次触发播一个样点并请求 DMA 预载下一个
//! （`write_dma`，DMA 通道 2）。32 点三角波 = 32ms 一轮，PC13 每轮翻转。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch11_dac_wave -p cw32-book-embassy
//! 状态：仅编译未上板（embassy 版 dac_wave 同状态；DAC 的 DMA 请求搬运
//!       语义出自手册 §26.7，但 TRGO 供拍 × 请求线 × 末样点补播时序未上
//!       板核对）。首次上板核对点见第 11 章正文。

#![no_std]
#![no_main]

use embassy_executor::main;
use embassy_time::{Duration, Timer, with_timeout};

use cw32_book_embassy::board;
use embassy_cw32::{
    Config, Peripherals,
    dac::{Dac, DacChannel, DacError, DacTrig},
    pac,
};

embassy_cw32::bind_interrupts!(struct Irqs {
    DMA12 => embassy_cw32::dma::Dma12Irq;
    DMA34 => embassy_cw32::dma::Dma34Irq;
});

/// 32 点三角波（0→4095→0，上坡 16 点、下坡镜像；1kHz 供拍下 32ms 一轮）
static TRIANGLE: [u16; 32] = {
    let mut a = [0u16; 32];
    let mut i = 0;
    while i < 32 {
        let k = if i < 16 { i } else { 31 - i };
        a[i] = (k as u32 * 273) as u16; // 峰值 15×273=4095
        i += 1;
    }
    a
};

/// 完成的整段播放轮数（心跳：每轮一次 DMA TC 唤醒）
#[used]
#[unsafe(no_mangle)]
pub static mut ROUNDS: u32 = 0;
/// 错误码：0=无 1=超时（触发源停拍？） 2=DMA 下溢 3=DMA 通道错误
/// 4=值越界 5=缓冲长度非法
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;
/// 最后一轮播放完的 DOR1（预期恒 0 = 三角波终点，末样点补播生效）
#[used]
#[unsafe(no_mangle)]
pub static mut LAST_DOR: u32 = 0;

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // GPIOC 时钟 + PC13 LED 心跳：门控 + 构造在 board::led() 里
    let mut led = board::led();

    // ---- GTIM1 → TRGO 供拍（1kHz 更新；表 26-1 TSEL=0100 = GTIM1_Trgo）----
    // APBEN1.GTIM1（bit6，KEY 读改写——保留其它外设使能位）
    p.sysctrl.apben1.modify(|v| {
        pac::sysctrl::KEY | (v & 0xFFFF) | pac::sysctrl::APBEN1_GTIM1
    });
    let gtim1 = p.gtim1;
    gtim1.cr1.write(0); // 停止计数以便配置
    // PSC=3、ARR=999：PCLK 4MHz /(3+1)/(999+1) = 1kHz 更新
    gtim1.psc.write(3);
    gtim1.arr.write(999);
    // CR2.MMS[6:4]=010（update → TRGO；PAC 未收编该位域故写裸值）
    gtim1.cr2.write(2 << 4);
    gtim1.egr.write(pac::gtim::EGR_UG); // 重装影子寄存器
    gtim1.cr1.write(pac::gtim::CR1_EN); // 跑起来，开始供拍

    // ---- DAC + DMA 波形链（默认不使能引脚输出：PA4 与 SPI1 引脚冲突；
    //      需要示波器看波形时补一行 dac.enable_pin1()，探头接 PA4）----
    let mut dac = Dac::new(p.dac);
    let mut dma = embassy_cw32::dma::Dma::new(p.dma, Irqs);
    dac.set_trigger(DacChannel::Ch1, DacTrig::Gtim1Trgo);

    loop {
        // 32 样点 @1kHz = 32ms/轮；2s 超时兜底（触发源停拍时不挂死）
        let r = with_timeout(
            Duration::from_millis(2000),
            dac.write_dma(&mut dma, DacChannel::Ch1, &TRIANGLE),
        )
        .await;
        match r {
            Ok(Ok(())) => unsafe {
                ERR = 0;
                LAST_DOR = dac.output1() as u32;
            },
            Ok(Err(DacError::DmaUnderrun)) => unsafe { ERR = 2 },
            Ok(Err(DacError::Dma(_))) => unsafe { ERR = 3 },
            Ok(Err(DacError::ValueOutOfRange)) => unsafe { ERR = 4 },
            Ok(Err(DacError::BadLength)) => unsafe { ERR = 5 },
            Err(_) => {
                // 超时：触发链没跑起来/请求没到——停通道收尾再重试
                dac.stop_dma(&mut dma, DacChannel::Ch1);
                unsafe { ERR = 1 }
            }
        }
        unsafe {
            ROUNDS += 1;
            led.toggle();
        }
        Timer::after_millis(468).await;
    }
}
```

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch11_dac_wave -p cw32-book-embassy
```

## 板上验证

- **ch11_adc：仅编译未上板。** 与 embassy-cw32 仓的 `adc_async` 示例
  同源；VDDA/内部通道的寄存器配方与阻塞版 `adc_internal`/`adc_sequence`
  同源且已实机验证（hal 仓 TESTING.md：VDDA=3282 mV、BGR 原始码 1488
  ≈理论值 1489）。首次上板核对：`RESULTS[0]` 落在 3200~3300 mV、
  `RESULTS[1]` 随 PA0 电平变化、`ERR=0`；embassy 异步路径（EOC/EOS
  中断唤醒）本身待板核对。
- **ch11_dac_wave：仅编译未上板。** 与 embassy-cw32 仓的 `dac_wave`
  示例同源，且其源码注释明确标注"DAC DMA 模式为推断实现，待板验证"：
  §26.7 的请求搬运语义有手册原文支撑，但 `hs::DAC1` 请求线 × GTIM TRGO
  供拍 × 末样点补播的时序从未上板核对（阻塞版没有 DAC DMA 配方）。
  首次上板核对：`ERR=0`、`ROUNDS` 递增（触发链活着）、`LAST_DOR` 恒 0
  （补播生效）；示波器接 PA4 应看到 31 Hz 三角波（1 kHz ÷ 32 点）。
  若 `ERR=2`（DMA 下溢）说明触发快于 DMA，若 `ERR=1`（超时）说明请求
  链没通——保守回退路径是把 `write_dma` 换成定时器中断里直写 DHR。

## 常见坑

- **ADC2 没有内部通道**：对 `new_adc2` 构造的实例调 `vdda_mv()`/
  `temperature_mc()` 返回 `AdcError::UnsupportedChannel`。
- **外部引脚忘了 `into_analog()`**：数字输出驱动器会把引脚压在 0V，
  读数恒 0——hal 仓 analog_loop 例程的实测教训。
- **序列通道数 2..=8**：单通道用 `read`，序列至少两通道；`out` 缓冲
  长度不足会整体拒绝。
- **`read_sequence_dma` 的配方是单次路径的推广**：阻塞版实机验证过的
  是单次转换 DMA（DMAEOC + `ADC1_SINGLE`），序列整块搬运（DMAEOS +
  `ADC1_SEQ`，一次请求搬 REPEAT 个元素）是按手册数据块模型推广的——
  首次上板请核对缓冲内容与逐通道读回是否一致。
- **DAC 通道使能与 SPI 引脚冲突**：`enable_pin1/enable_pin2` 会把 PA4/
  PA5 切给 DAC，若同板还要用 SPI1 需二选一；内部互联（ADC 回环）不受
  引脚影响。
- **`write_dma` 无内置超时**：触发源停拍会永久挂起，例程用
  `embassy_time::with_timeout` 包了一层兜底。
