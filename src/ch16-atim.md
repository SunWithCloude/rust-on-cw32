# 第 16 章 互补 PWM 与刹车:ATIM 高级定时器

这一章你会学到:

- 为什么半桥电路需要**互补 PWM + 死区**——以及"直通短路"这个烧管子的事故是怎么发生的;
- ATIM 相对 GTIM 多出来的三件事:**MOE 主输出使能**、**BDTR 死区发生器**、**刹车 BRK**;
- DTG 死区编码的四段规则,以及 `encode_dtg` 如何把"250µs"翻译成 0xFF;
- 刹车引脚的正确打开方式——一个实机抓到的"输出无波形"大坑。

本章例程(阻塞 HAL 版)对应 hal 仓 TESTING.md 的 `atim_deadtime` 条目,
是**实机验证过的配方**:BDTR 读数、刹车熄灭、CNT 铺满全部有实测数据。
embassy 版 `atim` 驱动已合入 embassy-cw32(main 分支),本章阻塞版与
async 版两个例程都有;0.4.0 正式发布后书例程的 git 依赖切版本依赖。

## 单路 PWM 接不动半桥

第 10 章的 GTIM 输出的是一路"高电平占空比可调"的方波,点亮 LED、调
舵机都够用。但驱动电机功率级时,主流拓扑是**半桥**:上管 Q1 接电源正,
下管 Q2 接地,负载接在两者中间。

```text
        VCC          VCC
         |            |
        [Q1]        [Q2]   ← 不能同时导通!
         |____________|
              |
            负载
```

Q1 和 Q2 **绝不允许同时导通**——那等于把电源正和电源地直接短接
("直通"),功率管瞬间烧毁。所以控制信号必须满足:

- **互补**:Q1 关断的同时 Q2 导通,负载上才有电流回路;
- **死区**:但"同时"在物理上做不到无缝衔接——开关管的关断有延迟,
  如果 Q1 还没完全关断 Q2 就导通,直通照样发生。于是两路信号之间要
  留出一段**两路都关断**的间隔(死区),通常几百 ns 到几 µs。

GTIM 做不了这件事:它每通道只有一路输出。ATIM(高级定时器)每通道
有一对互补输出 CHy/CHyN,并在硬件里**统一插入死区**——软件只管写
占空比,死区由 DTG 寄存器一次性配好。

## ATIM 在 GTIM 基础上加了什么

ATIM 的计数主体(PSC/ARR/CCR/影子寄存器/ARPE 预装载,§17.1)与
GTIM 完全同构,第 10 章讲的模型原样适用。多出来的东西集中在三个
寄存器:

| 能力 | 寄存器/位 | 语义 |
|------|-----------|------|
| 主输出使能 | BDTR.MOE | **所有 PWM 引脚的总开关**。不置 1,CCER 配得再对引脚也没波形 |
| 死区发生器 | BDTR.DTG[7:0] | 一对互补输出间的双边关断间隔,硬件插入 |
| 刹车 BRK | BDTR.BKE/BKP/BKF | 外部引脚有效时**硬件异步清 MOE**,输出立即进安全态 |
| 空闲态输出 | BDTR.OSSI/OSSR | 刹车/空闲时输出"已定义电平"而不是高阻 |
| 重复计数 | RCR | 每 N 个周期才产生一次更新事件(三相同步采样用) |

关键理解:**MOE 是最后一道闸门**。初始化顺序通常是——引脚 AF 复用 →
PSC/ARR/CCR → CCER 使能通道 → DTG 死区 → 最后置 MOE。上一电就有波形,
中间任何一步出错都不至于把半桥打穿(如果配了刹车,还能靠 BRK 兜底)。

阻塞 HAL 把这套顺序封装在 `Atim::start_pwm_complementary` 里,调用者
一次给齐频率/占空比/死区即可;`start_pwm` 则是无互补的单端版本(同一
个定时器,不想接半桥时也能当高级版 GTIM 用)。

## 死区时间:DTG 四段编码

CW32L012 的 DTG 编码(手册 §17,与 STM32 高级定时器同款思路):

| DTG[7:5] | 死区时间(以 t_DTS 为单位) |
|----------|---------------------------|
| 0xx      | DTG[7:0] × t_DTS(1..127,步进 1) |
| 10x      | (64 + DTG[5:0]) × 2 × t_DTS(128..254,步进 2) |
| 110      | (32 + DTG[4:0]) × 8 × t_DTS(256..504,步进 8) |
| 111      | (32 + DTG[4:0]) × 16 × t_DTS(512..1008,步进 16) |

`t_DTS = 1/f_DTS`,`f_DTS = PCLK / CKD`(CR1.CKD,死区不走 PSC——
PSC 只分频计数器,不分频死区时钟;HAL 固定 CKD=1,即 t_DTS = t_PCLK)。

三件事值得注意:

- **死区不是线性连续的**:小死区分辨率 1 t_DTS,大死区步进放宽到
  2/8/16 t_DTS。这是"8 位寄存器表达 1008 档"的压缩技巧;
- **上限 1008 t_DTS**:4 MHz PCLK 下就是 252 µs。再长就配不出来,
  `encode_dtg` 返回 `None` → `AtimError::DeadTimeTooLong`;
- **向上取整**:HAL 的 `encode_dtg(pclk_hz, ckd_div, dead_ns)` 把请求
  的纳秒值**向上**取到最近可编码档——死区宁长勿短,短了直通,长了
  只是效率略降。例程里请求 250 µs,实际编码到 0xFF 档(1008 t_DTS ≈
  252 µs)。

## 刹车 BRK:硬件异步清 MOE

刹车是 ATIM 的"急停按钮":BDTR.BKE 使能后,BRK 引脚(极性由 BKP 定)
一旦有效,硬件**立刻**清 MOE——不经过软件,不等待更新事件,所有
PWM 引脚按 OSSI 定义的安全态输出。软件要恢复,调用
`re_enable_output()` 重置 MOE(BDTR.BIF 标志同时可读:刹车发生过,
BIF=1;BRK 源还活跃时清不掉)。

**实机大坑**(hal 仓 TESTING.md 原文):刹车输入脚 PA06 必须
`into_af_input(Af::Af7)`——走 **AF 复用输入**进定时器的刹车电路。
普通 GPIO 输入或悬空会让 BRK 采样为低 → 上电即刹车锁存 → MOE 被
硬件清零 → 引脚无波形,BDTR 读回 0x14FF 而不是预期的 0x94FF。排了
半天"为什么没波形",答案是引脚模式差一格。

## 例程

`examples/hal/examples/ch16_atim.rs`:PA08=CH1 / PA15=CH1N(均 AF7,
手册表 9-2)输出 2 kHz 互补 PWM,30% 占空比,死区 250 µs(编码到
0xFF);PA06=BK 配成 AF7 复用输入,上拉,低电平有效刹车。观测量全
部落在静态量里,probe-rs 可读:

```rust
//! 第 16 章例程:ATIM 互补 PWM + 死区 + 刹车(PA08=CH1、PA15=CH1N、PA06=BK)。
//!
//! 与 cw32l012-hal examples/atim_deadtime.rs 同源(仅静态量加
//! `#[used]`+`#[unsafe(no_mangle)]` 便于 probe-rs 直接读符号,并按
//! edition 2024 调整属性写法)。2kHz、30% 占空比、死区 250µs(4MHz
//! PCLK 下向上取整到 DTG=0xFF,即 1008 t_DTS ≈ 252µs)。
//!
//! 验证方式(无示波器也可,接调试器读静态量):
//! - `REG` 回读 (ARR<<16 | PSC)——预期 0x07CF_0000(ARR=1999、PSC=0)
//! - `CCR` 回读——预期 600(2000×30%)
//! - `BDTR` 回读——预期 0x94FF(MOE=1、OSSI=1、BKE=1、DTG=0xFF)
//! - `CNT_MIN/MAX`——预期铺满 0..1999
//! - 接示波器:PA08/PA15 互补方波,翻转沿间隔 ≈252µs 互不重叠;
//!   拉低 PA06(ATIM_BK)两路同时熄灭(MOE 被硬件清零,`BDTR` 读回
//!   0x14FF),`re_enable_output()` 恢复。
//!
//! 构建:cargo build --release --target thumbv6m-none-eabi --example ch16_atim -p cw32-book-hal
//! 状态:同源已验证——对应 hal 仓 TESTING.md 实测通过表 atim_deadtime 条目
//!       (REG=0x07CF0000、CCR=600、BDTR=0x94FF、CNT 铺满、PA06 刹车熄灭);
//!       本组合文件本身仅编译未上板。

#![no_std]
#![no_main]

use cw32_book_hal as _; // 继承 lib.rs 的 panic handler(panic-probe 同款机制)
use cw32l012_hal::{
    atim::{Atim, AtimChannel},
    gpio::{Af, Pin, PortId},
    rcc::Rcc,
    Peripherals,
};

/// (ARR<<16 | PSC) 配置回读:2kHz@PCLK 4MHz 预期 0x07CF_0000
#[used]
#[unsafe(no_mangle)]
pub static mut REG: u32 = 0;
/// 最近一次 CCR 回读:30% 占空比预期 600
#[used]
#[unsafe(no_mangle)]
pub static mut CCR: u16 = 0;
/// BDTR 回读(MOE/DTG/刹车位):预期 0x94FF
#[used]
#[unsafe(no_mangle)]
pub static mut BDTR: u32 = 0;
/// CNT 采样最小值:预期 0(计数器真的在滚)
#[used]
#[unsafe(no_mangle)]
pub static mut CNT_MIN: u16 = 0xFFFF;
/// CNT 采样最大值:预期 1999
#[used]
#[unsafe(no_mangle)]
pub static mut CNT_MAX: u16 = 0;

#[unsafe(no_mangle)]
fn main() -> ! {
    let p = match Peripherals::take() {
        Some(p) => p,
        None => loop {
            core::hint::spin_loop();
        },
    };

    let mut rcc = Rcc::new(p.sysctrl, p.flashctrl);
    let clocks = rcc.clocks(); // PCLK = 4MHz

    // 互补输出引脚:PA08 = ATIM_CH1、PA15 = ATIM_CH1N(均 AF7,手册表 9-2)
    let _ch1 = Pin::new(p.gpioa, PortId::A, 8, &mut rcc).into_af_output(Af::Af7);
    let _ch1n = Pin::new(p.gpioa, PortId::A, 15, &mut rcc).into_af_output(Af::Af7);
    // 刹车输入 PA06 = ATIM_BK:必须走 AF7 复用输入(内部有滤波/极性电路,
    // 普通 GPIO 输入模式采样不到 BRK);外部上拉保持高电平 = 不刹车,
    // 拉低 = 刹车。悬空会让 BRK 随噪声误触发(实机抓到的坑:普通输入
    // 模式下 MOE 被清,输出无波形,BDTR 读回 0x14FF)
    let _bk = Pin::new(p.gpioa, PortId::A, 6, &mut rcc).into_af_input(Af::Af7);

    let mut tim = Atim::new(p.atim, &mut rcc);
    // 2kHz @ PCLK 4MHz:PSC=0、ARR=1999;30% 占空比;死区 250µs
    // (encode_dtg 向上取整 → DTG=0xFF,1008 t_DTS ≈ 252µs)
    if tim
        .start_pwm_complementary(AtimChannel::Ch1, clocks.pclk_hz, 2_000, 3_000, 250_000)
        .is_err()
    {
        loop {
            core::hint::spin_loop();
        }
    }
    // 刹车:BRK 低电平有效(外部拉低 → MOE 硬件清零 → 输出安全态)
    tim.enable_brake(false, 0);
    tim.re_enable_output(); // 若上电瞬间有毛刺已锁存刹车,此处恢复主输出

    unsafe {
        let (arr, psc) = tim.period_config();
        REG = (arr as u32) << 16 | psc as u32;
        CCR = tim.ccr(AtimChannel::Ch1);
        BDTR = tim.bdtr();
    }

    // 运行观测:CNT 滚动范围(证明计数器活着);需要时也可在此周期性
    // set_duty 做呼吸——占空比下一周期才生效(OCxPE 预装载)
    loop {
        unsafe {
            let c = tim.count();
            if c < CNT_MIN {
                CNT_MIN = c;
            }
            if c > CNT_MAX {
                CNT_MAX = c;
            }
        }
    }
}
```

预期读数(全部有实机旁证,详见 hal 仓 TESTING.md):

| 静态量 | 预期 | 含义 |
|--------|------|------|
| `REG` | `0x07CF_0000` | ARR=1999、PSC=0(4M/2k = 2000 档) |
| `CCR` | `600` | 2000 × 30% |
| `BDTR` | `0x94FF` | MOE=1、OSSI=1、BKE=1、DTG=0xFF |
| `CNT_MIN/MAX` | `0` / `1999` | 计数器铺满整个周期 |

接示波器(可选):PA08/PA15 互补方波,两个翻转沿之间间隔 ≈252 µs
互不重叠;把 PA06 拉低,两路**同时**熄灭(BDTR 读回 `0x14FF`,MOE
位没了);调 `re_enable_output()` 恢复。

## 常见坑

- **刹车脚悬空/普通输入模式**:上电即锁存刹车、MOE 被清、输出无
  波形(BDTR 读回 0x14FF)。必须 `into_af_input(Af::Af7)` 且外部
  上拉(低有效配置时)。
- **死区请求超上限**:请求 > 1008 t_DTS 会返回 `DeadTimeTooLong`,
  不是静默钳位到最大档——宁报错勿将就,半桥参数错了烧的是管子。
- **MOE 忘了置**:引脚没波形先查 BDTR.MOE。ATIM 的输出链是
  比较器 → CCER → 死区 → **MOE** → 引脚,前三级全对,末级闸门没开
  一样是平线。
- **BDTR 整字写覆盖**:刹车配置(BKE/BKP/BKF)和死区(DTG)、总开关
  (MOE)同住一个寄存器,后写的整字写会把先写的位顶掉。HAL 内部用
  读改写(`modify`),自己摸寄存器时要记住 BDTR 是"多户合住"。

## embassy 版:三相互补 + 异步周期节拍

`embassy-cw32`(git 依赖,main 分支)的 `atim` 模块把同一套 ATIM 能力
带进异步世界:`AtimPwm::new` 一次收齐**三相互补对 + CH4 单端 + 刹车脚**,
构造时 MOE 默认关(输出空闲全低),显式 `enable()` 才有波形;
`wait_update().await` 用更新中断(IRQ13)把 PWM 周期边界变成异步节拍
——与第 10 章 GTIM 的 `wait_update` 同一模式,等待期间才开 UIE,
唤醒即压掉(防中断风暴)。刹车路径也是异步友好的:主循环轮询
`is_braked()`,源释放后 `clear_break()` + `enable()` 显式恢复
(构造参数 `aoe: false` 时硬件不自动恢复,决策权留给上层)。

例程 `examples/embassy/examples/ch16_atim_async.rs`:20kHz 三相互补
(六路,PA8/PA7、PA9/PB0、PA10/PB1,全 AF7)、1µs 死区(4MHz 下 DTG=4)、
占空比 0→100%→0 三角斜坡且三路错 120°(粗略旋转矢量观感,非正弦
SVPWM),PA6=BRK 低电平有效刹车。

```rust
//! 第 16 章例程(embassy 版):ATIM 三相互补 PWM + 死区 + 刹车。
//!
//! 与 embassy-cw32 examples/atim_motor.rs 同源(20kHz 三相互补、1µs 死区、
//! 三路占空比错 120° 三角斜坡、每个 PWM 周期 `wait_update().await` 节拍、
//! 刹车监控 + 显式恢复)。注意这不是正弦 SVPWM——示例只演示互补输出、
//! 死区与周期节拍,不做电机控制。
//!
//! # 接线(数据手册 V1.0 表 5-3,全部 AF7)
//!
//! | 信号 | 引脚 | 信号 | 引脚 |
//! |------|------|------|------|
//! | CH1  | PA8  | CH1N | PA7  |
//! | CH2  | PA9  | CH2N | PB0  |
//! | CH3  | PA10 | CH3N | PB1  |
//! | BK   | PA6  | (刹车输入,低电平有效,外部上拉) |
//!
//! 刹车路径:过流比较器(开漏)拉低 PA6 → 硬件**异步**清 MOE → 六路
//! 立即进空闲安全态(上/下管全关,不经软件/中断/死区逻辑)。ISR.BIF
//! 置位(`is_braked()`);刹车源活跃期间 BIF 清不掉(`clear_break()`
//! 返回 false)。排除故障 → `clear_break()` → `enable()` 手动恢复
//! (本例 `aoe=false`,恢复决策权留给上层)。
//!
//! # 验证方式(无示波器也可)
//!
//! - `PERIOD` 静态量记录 (PSC<<16 | ARR)——预期 PSC=0、ARR=199
//!   (20kHz @ PCLK 4MHz)
//! - `DTG` 记录 DTG 编码——1µs @ 4MHz = 4 tick,预期 0x04
//! - `UPDATES` ≈20000/s 线性增长(wait_update 真在等更新中断)
//! - `BRAKES`/`ENABLE_FAILS`:PA6 拉低再释放——BRAKES+1、
//!   ENABLE_FAILS+1(刹车压着时 enable() 置不上 MOE),释放并
//!   clear_break + enable 后恢复斜坡
//! - 示波器:PA8/PA7 互补方波 + 1µs 死区(交叠处两路同低),
//!   PA9/PB0、PA10/PB1 同理且错 120°
//!
//! 结果落 SRAM 静态量,`probe-rs read` 可读。
//!
//! 构建:cargo build --release --target thumbv6m-none-eabi --example ch16_atim_async -p cw32-book-embassy
//! 状态:仅编译未上板;驱动与阻塞版 atim 模块同源(阻塞版配方已在 hal
//!       仓 TESTING.md atim_deadtime 条目实机验证),embassy 版驱动
//!       compiles-not-run——上板前勿接真电机,先用逻辑分析仪小电压验证。

#![no_std]
#![no_main]

use embassy_executor::main;

use cw32_book_embassy as _; // 继承 lib.rs 的 panic handler(panic-probe 同款机制)
use embassy_cw32::{
    Config, Peripherals,
    atim::{AtimError, AtimPwm, AtimPwmConfig},
    gpio::Pin,
};

// wait_update 需要 ATIM 更新中断(IRQ13)
embassy_cw32::bind_interrupts!(struct Irqs {
    ATIM => embassy_cw32::atim::AtimIrq;
});

/// (PSC<<16 | ARR) 配置回读(预期 PSC=0、ARR=199)
#[used]
#[unsafe(no_mangle)]
pub static mut PERIOD: u32 = 0;
/// DTG 死区编码回读(预期 0x04 = 4 tick @ 4MHz 的 1µs)
#[used]
#[unsafe(no_mangle)]
pub static mut DTG: u32 = 0;
/// wait_update 完成次数(预期 ≈20000/s)
#[used]
#[unsafe(no_mangle)]
pub static mut UPDATES: u32 = 0;
/// 刹车进入次数(PA6 拉低事件)
#[used]
#[unsafe(no_mangle)]
pub static mut BRAKES: u32 = 0;
/// 刹车压着时 enable() 失败次数(MOE 置不上)
#[used]
#[unsafe(no_mangle)]
pub static mut ENABLE_FAILS: u32 = 0;
/// 构造错误码:0=无 1=FreqOutOfRange 2=DeadTimeTooLong 3=BadBreakConfig
/// 4=BadDuty
#[used]
#[unsafe(no_mangle)]
pub static mut ERR: u32 = 0;

/// 三角波占空比(万分比):0 → 10000 → 0,200 步一循环
fn ramp(step: u32) -> u16 {
    let s = step % 200;
    let v = if s < 100 { s * 100 } else { (200 - s) * 100 };
    (v.min(10_000)) as u16
}

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk/PCLK = 4MHz

    let p = Peripherals::take().unwrap();

    // GPIOA(bit4: PA6-PA10) + GPIOB(bit5: PB0/PB1) 端口时钟——KEY 保护
    // 寄存器整字写一次到位,分两次写会互相顶掉使能位(第 2 章同款坑)
    p.sysctrl
        .ahben
        .write(embassy_cw32::SYSCTRL_KEY | (1 << 4) | (1 << 5));

    // 三相互补 + 死区 + 刹车(引脚/AF 见文件头接线表)
    // 不用的通道传 None:APIT 端口类型无法从 None 推断,需显式标注
    let no_pin: Option<(Pin<embassy_cw32::Gpio>, u8)> = None;
    let pwm = AtimPwm::new(
        p.atim,
        AtimPwmConfig {
            freq_hz: 20_000,     // 20kHz 载波(4MHz PCLK 下 ARR=199)
            dead_time_ns: 1_000, // 1µs 死区(4 tick,向上取整)
            aoe: false,          // 刹车不自动恢复——clear_break + enable 显式来
        },
        Some((Pin::new(p.gpioa, 8), 7)),  // CH1  = PA8  AF7
        Some((Pin::new(p.gpioa, 7), 7)),  // CH1N = PA7  AF7
        Some((Pin::new(p.gpioa, 9), 7)),  // CH2  = PA9  AF7
        Some((Pin::new(p.gpiob, 0), 7)),  // CH2N = PB0  AF7
        Some((Pin::new(p.gpioa, 10), 7)), // CH3  = PA10 AF7
        Some((Pin::new(p.gpiob, 1), 7)),  // CH3N = PB1  AF7
        no_pin,                           // CH4 单端(本例不用)
        Some((Pin::new(p.gpioa, 6), 7)),  // BK   = PA6  AF7(低有效)
        false,                            // BKP=0:低电平有效
        0,                                // BKF=0:无数字滤波
        Irqs,
    );
    let mut pwm = match pwm {
        Ok(t) => t,
        Err(e) => {
            unsafe {
                ERR = match e {
                    AtimError::FreqOutOfRange => 1,
                    AtimError::DeadTimeTooLong => 2,
                    AtimError::BadBreakConfig => 3,
                    AtimError::BadDuty => 4,
                };
            }
            loop {
                core::hint::spin_loop();
            }
        }
    };

    let (psc, arr) = pwm.period_config();
    unsafe {
        PERIOD = (psc as u32) << 16 | arr as u32;
        DTG = pwm.dtg() as u32;
    }

    // 置 MOE:构造时 MOE=0(输出空闲全低)。BK 已活跃则置不上 → 返回 false
    if !pwm.enable() {
        unsafe { ENABLE_FAILS += 1 };
    }

    // 占空比 0→100%→0 三角斜坡,三路错 120°(万分比 3334/6667):
    // 非正弦,仅演示三相互补输出与 120° 相位观感
    let mut step: u32 = 0;
    loop {
        let d = ramp(step);
        pwm.set_duty_bp(1, d).ok();
        pwm.set_duty_bp(2, (d + 3_334) % 10_000).ok();
        pwm.set_duty_bp(3, (d + 6_667) % 10_000).ok();

        // 等一个真实 PWM 周期(ATIM 更新中断 + waker 异步路径)
        pwm.wait_update().await;
        unsafe { UPDATES += 1 };

        // 刹车监控:BIF 置位即六路输出已被硬件异步压到安全态(MOE=0)。
        // clear_break 在刹车源仍活跃时返回 false;源释放后重新 enable。
        if pwm.is_braked() {
            unsafe { BRAKES += 1 };
            if pwm.clear_break() && !pwm.enable() {
                unsafe { ENABLE_FAILS += 1 };
            }
        }
        step += 1;
    }
}
```

预期读数:`PERIOD` = `0x00C7_0000`(PSC=0、ARR=199)、`DTG` = `0x04`、
`UPDATES` ≈ 20000/s。PA6 拉低再释放可观察 `BRAKES`/`ENABLE_FAILS`
递增与斜坡恢复。

⚠ 状态标注:**embassy 版驱动 compiles-not-run**(实机验证待板)——上板
前勿接真电机,先用逻辑分析仪 + 小电压验证死区与刹车时序。阻塞版配方
(`cw32l012-hal` `atim_deadtime`)是实测过的,寄存器语义两版同源。

## 思考题

1. 若把例程的死区请求改成 300 µs(> 1008 t_DTS = 252 µs 上限),
   `start_pwm_complementary` 返回什么?如何在 4 MHz PCLK 下合法获得
   300 µs 死区?(提示:CKD 可以除 2/4,`encode_dtg` 的第二个参数)
2. BDTR.AOE(自动输出使能)=1 时,刹车恢复后下一个更新事件会自动
   重置 MOE。对电机应用,你倾向 AOE=0(必须软件显式恢复)还是 1?
   各自的风险是什么?
3. RCR(重复计数)=4 时,更新事件频率变成原来的 1/5。这对三相
   FOC 控制的"每周期集中刷新占空比"有什么好处?
