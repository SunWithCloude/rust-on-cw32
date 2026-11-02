# 写作约定（贡献者/作者必读）

本文件是章节作者的**硬约束**。mdBook 的正文在 `src/`，可编译例程在  
`examples/`，两者的对应关系与写作规范如下。

## 1. 文件所有权矩阵

| 部分     | 章节（src/）                                                              | 例程（examples/…/examples/）                          | 例程 crate         |
| ------ | --------------------------------------------------------------------- | ------------------------------------------------- | ---------------- |
| 起步     | index.md, ch01-setup.md, ch02-blink-pac.md, ch03-rtt.md               | ch02_blink.rs, ch03_rtt.rs                        | examples/pac     |
| 阻塞 HAL | ch04-hal-intro.md, ch05-hal-uart.md, ch06-hal-more.md                 | ch04_blink_pc13.rs, ch05_uart_echo.rs, ch06\_*.rs | examples/hal     |
| 异步     | ch07-async-intro.md, ch08-async-gpio-uart.md, ch09-bind-interrupts.md | ch07_blink_async.rs, ch08\_*.rs, ch09\_*.rs       | examples/embassy |
| 外设进阶   | ch10-pwm.md … ch14-i2c-spi.md                                         | ch10\_*.rs … ch14\_*.rs                           | examples/embassy |
| 共享封装层 | 各章"例程全文/工程化"小节嵌入的 `src/lib.rs`、`src/board.rs` 代码块 | examples/…/src/（lib.rs、board.rs：延时、Led、panic handler） | examples/{pac,hal,embassy} |
| 附录     | app-a…app-d.md                                                        | 无                                                 | 无                |

- **只动自己名下的文件**。`SUMMARY.md`、`book.toml`、根 `Cargo.toml`、  
  `CONVENTIONS.md`、CI 不许碰。
- 例程 crate 自己的 `Cargo.toml` 可以加 `[[example]]` 条目与 dev-dependencies  
  （只允许在自己的 crate 目录内改）。
- 新增例程后**必须**跑通再写正文（见 §3）。

## 2. 语言与风格

- 简体中文正文；API 名、寄存器名、命令行保持英文原样；代码注释用中文。
- 每章结构固定：
  1. 开头 3–5 条"这一章你会学到"；
  2. 正文（段落要短，一个概念一段）；
  3. 例程代码 + 构建命令；
  4. **"板上验证"状态**（见 §4）；
  5. "常见坑"小结（列表，每条一两句）。
- 寄存器/位段提及处标注手册章节（如 §25.4、§16.10.6），手册 =  
  《CW32L012 用户手册》V1.4。
- 不写营销话，不写"简单易懂"之类空评；结论给出处。

## 3. 例程纪律（最重要）

1. **先例程后正文**：先写 `chNN_xxx.rs`，跑到  
   `cargo build --release --target thumbv6m-none-eabi --example chNN_xxx`  
   通过，再写章节，**代码块内容与例程文件一字不差**。
2. **API 名必须来自 crate 源码**：拿不准的方法名/参数，去  
   `Desktop/cw32l012-pac|cw32l012-hal/cw32l012-hal|embassy-cw32` 的源码里  
   grep，或读其 docs；**禁止凭印象发明 API**。
3. 例程头注释写三行：本章号与主题、构建命令、板上状态（见 §4）。
4. edition 2024：裸函数符号用 `#[unsafe(no_mangle)]`；  
   `static` 被优化掉时用 `#[used]`。
5. 例程要**最小可跑**：一个例程只演示一件事；错误处理用最简形式  
   （`unwrap`/`match Ok`）但不得吞错误不解释。

## 4. 诚实标注规则

每个例程的"板上验证"状态三选一，**禁止编造**：

- **已实机验证**——有实板记录（引 cw32l012-hal `TESTING.md`、  
  embassy-cw32 CHANGELOG 或例程注释中的实测数据）；
- **同源已验证**——例程与生态仓已验证例程逐行同源（说明来源）；
- **仅编译未上板**——如实写，并在正文提示首次上板要核对什么。

## 5. 硬事实（禁止改动或臆造）

写作时可直接引用，但**不许**"顺手修正"这些结论——它们来自手册与实机：

- 复位默认时钟：L012 = **4 MHz**（HSI），HCLK = PCLK = 4 MHz；  
  经典家族（F030 等）= 8 MHz。
- L012 的 SYSCTRL 门控写保护：KEY = **0x5A5A**（AHBEN/APBEN 带键写）。
- 板载 LED：用户板 = **PC13**；UART1 = **PA02(TX)/PA03(RX), AF1**（实测）。
- 复位默认 PC13 处于模拟态（GPIOx_ANALOG=1），点灯前必须切数字功能。
- 常用中断号：GPIOA/B/C/F = 5/6/7/8，UART1/2/3 = 27/28/29，I2C1 = 23，  
  SPI1 = 25，ADC = 12。
- probe-rs 芯片名：**CW32L012x8**（上游 0.33+ 开箱可识别）。
- 已知坑（写进相关章节/附录 C）：F030 复位默认 ANALOG 仅 SWD 数字使能；  
  调试中 SWD 楔死（SwdApWdataError）先给目标板断电重插；debug 构建会优化  
  掉未引用的 static（`#[used]` 锁符号）；embassy-cw32 与 cw32l012-hal 的  
  critical-section 实现不能在同一固件共存（本书用独立例程 crate 隔离）。

## 6. 本地构建

```bash
cargo build --workspace --target thumbv6m-none-eabi --release --examples
mdbook build          # 需要 cargo install mdbook
```

生态依赖按路径挂在本仓上一级（`../../..`）：`cw32l012-pac`、  
`cw32l012-hal/cw32l012-hal`、`embassy-cw32`。
