# 前言

这本书教你在**武汉芯源（CMSemicon）CW32 系列**单片机上用 Rust 做开发——把
**CW32L012**（ARM Cortex-M0+，64 KB Flash / 8 KB SRAM）当作教学用具，
从点亮一颗 LED 一路写到异步外设全家桶。

## 为什么是 CW32L012

- **够小，够真**：M0+ 核、64K/8K 的容量，和市面上"第一个单片机"的学习曲线
  完全一致；但它又有 DMA、双 ADC、DAC、RTC、看门狗这些"真外设"，学完能干活。
- **一条完整的 Rust 生态链**：寄存器层（`cw32l012-pac`，逐项转录自用户手册
  V1.4）、阻塞式 HAL（`cw32l012-hal`，自带运行时零依赖）、异步 HAL
  （`embassy-cw32`，embassy 执行器 + SysTick 时间驱动）。三层都是本书的主角，
  也是开源仓库 [SunWithCloude](https://github.com/SunWithCloude) 下的真实项目。
- **所有例程可编译**：书里出现的每一段代码都对应本仓库
  `examples/` 下的一个真实例程文件，由 CI 在 `thumbv6m-none-eabi` 上持续编译。
  实机验证过的例程会明确标注，只编译过没上板的也会明确标注——不糊弄。

## 你需要准备什么

- 一块 **CW32L012** 开发板（本书例程默认板载 LED 在 **PC13**、UART1 走
  **PA02(TX)/PA03(RX)**——引脚不同请按原理图改两行）
- 一个 **ST-Link**（或任何 CMSIS-DAP/DAP-Link）调试器，SWD 四根线接好
- 一台装着 Windows/Linux/macOS 的电脑，**Rust 由第 1 章负责装好**

## 怎么读这本书

- **第一部分（第 1–3 章）** 不依赖任何 HAL：装好工具链，用 PAC 裸寄存器点亮
  LED，再用 RTT 让芯片往电脑上打日志。理解这一部分，你就理解了后面所有封装
  的底层。
- **第二部分（第 4–6 章）** 上手阻塞式 `cw32l012-hal`：一行点灯、串口收发、
  定时器/DMA/RTC/看门狗速览。写法与 C 标准库最接近，适合快速把东西做出来。
- **第三部分（第 7–9 章）** 进入 embassy 异步世界：执行器、时间驱动、中断
  绑定。从这开始，"等待"不再占用 CPU。
- **第四部分（第 10–14 章）** 异步外设进阶：PWM、ADC/DAC、DMA、RTC、I2C/SPI，
  每章一个能跑的完整例程。
- **附录** 是工具箱：11 个家族的移植矩阵、生态总览、故障排查手册、
  probe-rs 速查。卡住的时候先翻附录 C。

## 约定

- 例程按章编号：`examples/<层>/examples/chNN_名称.rs`，每章开头给出构建命令。
- 寄存器与位段一律标注用户手册章节（如 §25.4），手册指
  《CW32L012 用户手册》V1.4。
- 命令行统一用 bash 风格；Windows 用户在 PowerShell 下执行等价命令即可。
- 本书所有例程基于 Rust 2024 edition；`#[unsafe(no_mangle)]` 这类写法
  是 edition 2024 的要求，遇到时正文会解释。

---

本书与生态仓库持续演进，发现问题请到
[仓库 issues](https://github.com/SunWithCloude/rust-on-cw32/issues) 提。
