# 第 7 章 异步世界入门

这一章你会学到：

- 阻塞式 `Delay` 与异步 `Timer::after(...).await` 在 CPU 层面的本质区别；
- embassy 执行器如何用 `async fn` 组织任务（`#[main]`、`#[task]`、`Spawner::spawn`）；
- embassy-cw32 的 SysTick 时间驱动为什么做成"动态单次闹钟"，空闲时 CPU 在做什么；
- `Config`/`init()` 在异步固件里扮演什么角色；
- 逐段读懂 `ch07_blink_async.rs`，并把它烧进板子看到 PC13 精确闪烁。

## 7.1 从"空转"到"挂起"

第二部分写 LED 闪烁时，主循环里躺着一行 `delay.delay_ms(500)`。`cw32l012-hal`
的 delay 模块给自己的定位是"**SysTick 忙等延时**"（`cw32l012-hal`
`src/delay.rs`）：它把 SysTick 配成自由运行的倒计数器，然后在循环里反复读
`COUNTFLAG`，数够节拍数才返回。在这 500ms 里，CPU 100% 的时间都在执行那条
轮询循环——哪怕换成定时器中断 + 标志位轮询，本质仍是"占着 CPU 等待"。

单任务时这没什么。想同时做两件事（闪 LED + 扫描按键），你要么手写状态机，
把每个任务切成小步子在主循环里轮流执行；要么上 RTOS，用任务切换的代价换
直观的顺序写法。

异步给出第三条路，它的核心承诺只有一句话：**等待的时候，把 CPU 让出去**。

```rust,ignore
Timer::after_millis(500).await;
```

这行代码不是循环查询。它做的事是：把"500ms 后的到期时刻"登记进定时器队列，
然后告诉执行器"到点前别再调度我"——当前任务在 `.await` 点挂起，执行器发现
没有别的任务可跑，就让内核睡（WFE 指令）。500ms 后定时器中断触发，唤醒
任务继续执行 `led.toggle()`。等待期间 CPU 的占用是零。

`async`/`await` 只是语言机制：`async fn` 会被编译成一个状态机（实现了
`Future`），每个 `.await` 点是状态机"可以让出"的位置。真正决定"什么时候
调度谁"的是**执行器**（executor）。Rust 标准库不带执行器——这正是 embassy
登场的地方。

## 7.2 embassy 执行器：任务即 Future

embassy 是 embedded-wg 生态下的事实标准异步运行时。对本书而言，你只需要
理解三个构件：`#[main]`、任务、`Spawner`。

**`#[main]`**。标注在异步入口函数上，宏展开后生成真正的入口
（cortex-m-rt 的 entry）：创建一个执行器实例，把你的 async main 作为第一个
任务挂进去，然后进入调度循环。embassy-executor 在 Cortex-M 平台上的空闲
策略写在源码里：没有可跑的任务就执行 `asm!("wfe")` 睡眠，中断到来时被唤醒
（embassy-executor 0.10 `src/platform/cortex_m.rs`）。这就是"等待不占 CPU"
的最后一块拼图——不只是任务在等，整个内核都睡了。

**任务**。一个任务就是一个被 `#[embassy_executor::task]` 标注的 `async fn`。
宏会把它改写成一个普通函数，返回
`Result<SpawnToken<impl Sized>, SpawnError>`（embassy-executor-macros 0.8
`src/macros/task.rs`）——调用它只是"领取一张任务令牌"，真正把任务交给
执行器的是下一步：

```rust,ignore
spawner.spawn(echo(rx, tx).unwrap());
```

`unwrap()` 处理的是"令牌领取失败"：一个任务默认只允许一个实例在跑（任务
静态存储只有一份），重复 spawn 会得到 `SpawnError::Busy`（需要多实例时给
`#[task]` 加 `pool_size = N`，代价是按份数占用 RAM）。`spawner.spawn(token)`
把任务挂进执行器队列，等它第一次被 poll。

**所有权**。任务是 `'static` 的——执行器不知道你的任务什么时候结束，所以
传进任务的参数必须是值本身（引脚半部、`BufferedUartRx` 等），不能是栈上
变量的引用。这就是后面例程里 `u.split()` 之后把 `(tx, rx)` 两个**所有权
半部**直接 move 进 `echo` 任务的原因。

任务内部的等待靠 waker：任务在 `.await` 点把自己的 waker 注册给"事件的
提供方"（时间驱动的定时器队列、UART 中断的 waker 表、GPIO 的 per-pin
waker 表），然后返回 `Pending`；中断发生时事件提供方调用 `wake()`，执行器
在下轮调度里重新 poll 这个任务。第 8 章你会看到这套机制在 GPIO/UART 上的
具体接线。

## 7.3 SysTick 时间驱动：动态单次闹钟

`Timer::after_millis(500)` 能准点唤醒，靠的是 embassy-cw32 内置的时间驱动。
embassy-cw32 的库文档对它的概括是（`src/lib.rs` 头注释）：

> `init`：SysTick 时间驱动（1MHz tick，动态单次闹钟，空闲不醒）

拆开讲：

**embassy-time-driver 0.2 接口**。embassy-time 只提供 `Instant`/`Timer`
这些类型，真正的"现在几点"和"到点叫我"由时间驱动实现——一个只有两个方法
的 `Driver` trait：`now()` 返回当前时刻，`schedule_wake(at, waker)` 登记
到期唤醒。embassy-cw32 用 `tick-hz-1_000_000` 固定 1MHz tick，即 `Timer`
的计数单位是微秒（`embassy-cw32/src/time_driver.rs`）。

**M0+ 的 SysTick 没有比较寄存器**。Cortex-M0+ 的 SysTick 是一个 24 位倒
计数器（ARMv6-M 架构参考手册 DDI 0419 §B3.3）：数到 0 置 `COUNTFLAG`/
触发中断，重载 `RVR` 再来。它天生只会"周期中断"，不会"在指定时刻中断"。

**动态单次闹钟**就是冲着这个限制去的：平时 SysTick 以最大周期（2^24
systick）自由跑；每当定时器队列的**队头 deadline 变化**，驱动把 `RVR`
重编程为"距最近一个到期时刻还剩多少 systick"，写 `CVR = 0` 重启计数——
一次性的闹钟。到期中断里派发到期的定时器，再续排下一个闹钟
（`time_driver.rs` 模块文档与 `set_alarm()`）。

**空闲不醒**。传统的 1ms 节拍中断，不管有没有定时器在跑，每毫秒都闯进 CPU
一次。这里没有固定节拍：队列空了就没有"下一个 deadline"要追，SysTick 退化
成最大周期自由跑，唯一的周期性事件是 24 位计数器的回绕（4MHz HCLK 下约
4.19s 一次），中断里只做簿记、不唤醒任何任务。对低功耗场景，这就是"空闲
时没有周期唤醒"的含义。

三个边界行为，写程序时用得上：

- **长限期退化**：deadline 超过 24 位（4MHz 下约 4.19s）时，先编程为最大
  周期，回绕中断里重新检查队列——`Timer::after_secs(10)` 依然精确，只是
  中间多了几次簿记中断。
- **不早到，可能晚到**：比 `MIN_PERIOD`（100 systick，4MHz 下 25µs）更近
  的闹钟最多晚这么多个 systick 触发，绝不会提前——早到会破坏 embassy-time
  的语义，晚到有界（`time_driver.rs` 的 `MIN_PERIOD` 注释）。
- **now() 单调不回退**：64 位软件扩展靠"读清 `COUNTFLAG`"折算，双读校验
  消除回绕窗口；重编程后的第一拍读数会短暂超前，由单调钳制压回
  （`now_systicks()`）。这套时序曾经出过一个真实 bug——重编程基数误存为
  `now - (period - 1)`，导致 `now()` 在一个周期内震荡、deadline 永远够
  不着、LED 常亮不闪；靠 probe-rs 读寄存器和 RAM 定位后修复（embassy-cw32
  README "How the time driver works" 的 hardware debugging note，
  2026-10-24）。引它在这里是想说明：这段代码的每个细节都有实机排障背书。

## 7.4 Config 与 init()

```rust,ignore
embassy_cw32::init(Config::default());
```

embassy-cw32 **不做时钟树配置**。`Config { hclk_hz, pclk_hz }` 只是向库
*声明*当前总线频率：SysTick 的 systick→微秒折算、UART 的波特率除法都依赖
它。`Config::default()` 跟随复位默认——L012 代组为 4MHz（HSI 96MHz / 24，
分频编码见手册 §4.7.4），HCLK = PCLK = 4MHz。固件若要变频，先用寄存器层
配好时钟，再以实际频率调 `init()`。

`init()` 启动 SysTick 时间驱动，必须是 `#[main]` 函数的第一行——任何带
时间依赖的任务开始跑之前它就得就位。

另一个全局约束也在这层：embassy-cw32 默认启用 PRIMASK 版的
critical-section 实现，而**一个固件只能有一个 critical-section 实现**——
不要把第二部分的 `cw32l012-hal`（阻塞 HAL）和 `embassy-cw32` 混进同一个
固件，链接会冲突。本书的做法是三部分各用独立的例程 crate
（`examples/pac`、`examples/hal`、`examples/embassy`）从依赖层面隔离。

## 7.5 逐段精讲 ch07_blink_async.rs

整个例程不到 40 行，下面是全文
（`examples/embassy/examples/ch07_blink_async.rs`）：

```rust
//! 第 7 章例程：异步 GPIO 闪烁（PC13，板上 LED 常见位置）。
//!
//! embassy 执行器 + SysTick 时间驱动 + 异步任务的最小闭环：
//! `Timer::after_millis(500)` 只是把任务挂回定时器队列，SysTick 闹钟
//! 到点再唤醒——等待期间 CPU 睡在执行器里（WFE），不做任何空转。
//!
//! 构建：cargo build --release --target thumbv6m-none-eabi --example ch07_blink_async -p cw32-book-embassy
//! 状态：同源例程（embassy-cw32 examples/blink_async）已实机验证
//! （2026-10-24：PC13 精确 500ms 翻转，SysTick 闹钟周期实测
//! 1,997,499 ≈ 2,000,000 systicks，出处见第 7 章正文）。

#![no_std]
#![no_main]

use embassy_executor::main;
use embassy_time::Timer;

use embassy_cw32::{
    Config, Peripherals,
    gpio::{Level, Output, Pin},
    sysctrl_enable_gpioc,
};

#[main]
async fn main(_spawner: embassy_executor::Spawner) {
    embassy_cw32::init(Config::default()); // 复位默认 SysClk = 4MHz

    let p = Peripherals::take().unwrap();
    sysctrl_enable_gpioc(p.sysctrl);

    let mut led = Output::new(Pin::new(p.gpioc, 13), Level::High);

    loop {
        Timer::after_millis(500).await;
        led.toggle();
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
```

构建命令：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch07_blink_async -p cw32-book-embassy
```

逐段看：

**头三行属性**。`#![no_std]`/`#![no_main]` 与自定义 `#[panic_handler]` 在
第 2 章已经逐个解释过，这里只提一处差异：panic 处理循环里用了
`core::hint::spin_loop()` 而不是空 `loop {}`——行为等价，语义上更明确。

**init**。`embassy_cw32::init(Config::default())` 按 4MHz 的复位默认时钟
启动 SysTick 时间驱动。这是 `#[main]` 函数的第一行，7.4 节讲过原因。

**Peripherals::take()**。PAC 生成的外设单例，`take()` 一次之后返回
`None`——保证每个外设寄存器块只有一个所有者。`unwrap()` 在这里不会炸：
每个核只有一次机会。

**GPIOC 门控**。

```rust,ignore
sysctrl_enable_gpioc(p.sysctrl);
```

L012 代组的 SYSCTRL 门控寄存器带写保护：写入时高 16 位必须是键值 `0x5A5A`，
否则整次写入无效（手册 §4.7 SYSCTRL 寄存器组）。`sysctrl_enable_gpioc`
做的就是把 `SYSCTRL_KEY | (1 << 6)`（AHBEN 的 GPIOC 位）一次性写进去。

这里埋着一个本章最重要也最容易踩的坑：**AHBEN 是"键控整字写"**。键在
寄存器的高 16 位里，使能位在低 16 位里，所以每次写入都覆盖整个寄存器。
`sysctrl_enable_gpioa()` 之后接着调 `sysctrl_enable_gpioc()`，后者会把
前者刚开的 GPIOA 位清掉。要开两个端口，必须把两个位拼在**同一次写**里——
第 8 章的 `ch08_gpio_wait` 就会这么做。

**点灯**。`Output::new(Pin::new(p.gpioc, 13), Level::High)`：`Pin::new`
绑定端口与引脚号，`Output::new` 清掉该脚的 `GPIOx_ANALOG` 位、方向置输出、
按初始电平走 BSRR/BRR 置位。ANALOG 这一步是 L012 实打实的坑：**复位默认
PC13 处于模拟态（`GPIOx_ANALOG.PIN13 = 1`）**，数字输入输出全被切断，不清
掉这位点不亮——第 4 章的阻塞 HAL 已经处理过同款问题（`cw32l012-hal`
`examples/blink_pc13.rs` 的头注释）。

**主循环**。

```rust,ignore
loop {
    Timer::after_millis(500).await;
    led.toggle();
}
```

每一圈：`Timer::after_millis(500)` 在定时器队列登记一个 500ms 后的
deadline，任务挂起；执行器 WFE 睡眠；500ms 后 SysTick 闹钟中断派发唤醒，
`toggle()` 走 TOG 寄存器原子翻转（第 4 章）。换算一下：500ms = 2,000,000
个 systick（4MHz），在 24 位（16,777,216）上限之内，一次编程到位——这就
是下面板上要测的那个数。

## 7.6 烧录与观测

```bash
probe-rs download --chip CW32L012x8 \
  target/thumbv6m-none-eabi/release/examples/ch07_blink_async
probe-rs reset --chip CW32L012x8
```

芯片名 `CW32L012x8` 在 probe-rs 0.33+ 开箱可识别（附录 D 是 probe-rs 速查）。
没接逻辑分析仪的话，肉眼校验：亮灭各约半秒、周期 1 秒，与阻塞版看不出
差别——差别在 CPU：这段固件跑起来后，4MHz 的 CPU 在两个翻转之间是睡着的。

## 7.7 板上验证

embassy-cw32 README 的 "Verification status" 记录了这一例程的实机数据
（2026-10-24，probe-rs + ST-Link V2）：

- **blink_async 板上通过**：PC13 以干净的 1 秒周期翻转
  （`Timer::after_millis(500)` 驱动，经探针轮询 ODR 确认）；SysTick 闹钟
  周期实测 **1,997,499 ≈ 2,000,000 systicks**（500ms @ 4MHz HCLK）——
  异步执行器、时间驱动、任务唤醒整条链路端到端工作。

本仓库的 `ch07_blink_async.rs` 与该例程**代码逐行同源**（仅注释不同），
故本章例程标注为：**已实机验证（同源）**——验证数据出自生态仓
embassy-cw32 的上板记录。

## 7.8 常见坑

- **忘了 `init()` 或把它放在别处**：时间驱动没启动，`Timer` 全部错拍甚至
  挂死。`init()` 固定写在 `#[main]` 第一行。
- **变频后没同步 `Config`**：库不配时钟树，你改了 HCLK 却仍传 4MHz，
  Timer 与波特率全部错拍；变频后以实际频率重新 `init`。
- **PC13 点不亮**：复位默认模拟态（`GPIOx_ANALOG.PIN13=1`）+ GPIOC 门控
  未开。本例程里 `Output::new`/`sysctrl_enable_gpioc` 已替你处理——换别的
  引脚时记得检查同样两步。
- **连开两个端口时钟被覆盖**：`sysctrl_enable_gpioX` 系列是键控整字写
  （高 16 位必须 0x5A5A），第二次写会把第一次的位清掉；两个端口要一次写齐
  （第 8 章例程演示）。
- **混用两个 critical-section 实现**：`embassy-cw32`（默认开启）与
  `cw32l012-hal` 的 `critical-section-impl` 不能同固件共存，链接期冲突；
  本书用独立例程 crate 隔离。
- **极短定时器晚到**：比 100 systick（4MHz 下 25µs）更近的闹钟最多晚这么
  多触发，不早到。对毫秒级的周期无影响。
