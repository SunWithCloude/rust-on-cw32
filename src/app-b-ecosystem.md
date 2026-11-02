# 附录 B 生态总览

本书用到的所有 crate 与工具都来自 [SunWithCloude](https://github.com/SunWithCloude)
下的开源仓库。这一附录给出全景：三层架构里每层是谁、发布状态如何、与厂商
C SDK 是什么关系。版本信息以各仓 README/CHANGELOG 的口径为准（本书写作时点）。

## B.1 三层架构

```
                    你的固件（本书 examples/ 三个 crate）
      ┌──────────────────────────┬─────────────────────────────┐
      │ 阻塞线（第 2–6 章）        │ 异步线（第 7–14 章）          │
      │ cw32l012-hal             │ embassy-cw32                │
      │ 阻塞驱动 + 自带 runtime   │ embassy-executor + SysTick  │
      │ 零外部依赖（默认）         │ 时间驱动，11 家族 feature    │
      │ 只覆盖 L012               │ 互斥选家族                   │
      └────────────┬─────────────┴──────────────┬──────────────┘
                   │ 内置 pac 模块（同源转录）     │
      ┌────────────┴────────────┐   ┌───────────┴──────────────┐
      │ cw32l012-pac            │   │ cw32-pacs（10 个 crate）  │
      │ 手写，逐项转录 UM V1.4   │   │ cw32-pac-gen 从厂商 SDK   │
      │ 与官方 SVD 逐项纠错      │   │ 头文件生成，workspace 管理 │
      └─────────────────────────┘   └──────────────────────────┘
        PAC 层：no_std、零运行时依赖、svd2rust 风格 API
        （Peripherals::take()/steal()、read()/write()/modify()）

      板级（可选）：cw32-bsp —— 11 家族 pins 全表 + bsp_pin! + blink 模板
      工具链（独立于固件层）：
        probe-rs ←—— cw32-probe-rs-compat（target YAML + cw32-detect 识别）
        cw32-tools（VS Code 插件：构建/烧录/可视化调试/Agent API）
```

（结构出处：cw32-pacs README "在 CW32 Rust 生态中的位置"、cw32-bsp README
"生态位置"两图合并重绘。）

两个要点：

- **PAC 层是双轨的**：L012 单独一个仓（先于 cw32-pacs 发布，已支撑 hal 与
  实机调试，发布节奏与寄存器补丁独立），其余 10 家族共用生成器
  `cw32-pac-gen`，输出风格与 L012 PAC 一致（出处：cw32-pacs README 家族矩阵
  下方注）。
- **两条 HAL 线并存、按需选一**（出处：embassy-cw32 README "Relationship to
  cw32l012-hal"）：

| | cw32l012-hal | embassy-cw32 |
|---|---|---|
| 模型 | 阻塞 + 自带运行时 | 异步 / embassy |
| 依赖 | 无（默认零依赖） | embassy 生态 |
| 向量表 | 内置（`UART1_irq` 风格） | cortex-m-rt + PAC `rt`（`UART1` 风格） |
| 家族覆盖 | 仅 L012 | 11 家族 |

⚠ 两条线的 `critical-section` 实现**不能在同一固件共存**（hal 的
`critical-section-impl` 与 embassy-cw32 默认实现同时开链接会失败）——本书
第 7 章起改用独立例程 crate 的原因，详见附录 C 条目 4
（出处：embassy-cw32 README；CONVENTIONS §5）。

## B.2 各仓角色与版本状态

| 仓库 | 角色 | crates.io | 怎么引入 | 版本口径 |
|------|------|-----------|----------|----------|
| `cw32l012-hal` | L012 阻塞 HAL + runtime | **已发布** | `cw32l012-hal = "0.8"` | 本地 Cargo.toml **0.9.1**；README 徽章指向 crates.io/docs.rs；v0.9 生态特性（critical-section/embedded-io/defmt）已实机验证 |
| `cw32l012-pac` | L012 PAC（手写） | **未发布**——名字被占 | path 依赖 | 本地 0.2.1；注册数据与 hal v0.9.1 的 pac 模块逐字节同源 |
| `cw32-pacs` | 其余 10 家族 PAC | **发布待定** | git 依赖 | crates.io 发布"待与原厂确认商标/命名合作事宜后定档" |
| `embassy-cw32` | 11 家族异步 HAL | **未发布** | git/path 依赖 | Cargo.toml 0.3.0；CHANGELOG 已定档 0.1.0/0.2.0（均 2026-10-24），"0.4.0 增量"段未发布 |
| `cw32-bsp` | 11 家族板级模板 | 未发布 | git 依赖（`cw32f030-bsp = { git = … }`） | 无版本号口径，README 给出 git 用法 |
| `cw32-probe-rs-compat` | probe-rs target YAML + 识别工具 | 工具仓（不发布库） | clone 使用 | `cw32-detect` 依赖 probe-rs 0.32.x 公开 API |

出处：cw32l012-hal README（crates.io 徽章、用法段、Roadmap v0.9 条）与
Cargo.toml；cw32l012-pac README（"Publishing note"、Verification 段）与
Cargo.toml；cw32-pacs README（"发布状态"）；embassy-cw32 Cargo.toml 与
CHANGELOG；cw32-bsp README（"用法"）；cw32-probe-rs-compat README
（"已知限制"）。

两个容易踩的名字陷阱：

1. **crates.io 上的 `cw32l012_pac` 0.1.1 不是本书用的仓**。那是 svd2rust
   v0.37.1 从厂商 SVD 生成的版本（2026-06-25 发布）。本书所用 `cw32l012-pac`
   因这个名字被占，将来发布需换名（如 `cw32l012-pac-um`）——path 依赖不受
   影响。SVD 版有三处已知缺陷（`GPIOx_PDR @ 0x0C` 缺失、`IRQ31 = FAULT`
   缺失、`SYSCTRL.PLL @ 0x28` 残留），手写 PAC 已按 UM V1.4 纠正
   （出处：cw32l012-pac README "Why not the cw32l012_pac 0.1.1"、
   cw32l012-hal README "Register-level confidence"）。
2. **`embassy-cw32` 的官方 GitHub 仓原名 `Cw32l012_embassy`**——cw32-bsp/
   cw32-pacs 的 README 链接仍指向该地址，仓库内容即 11 家族异步 HAL
   （出处：cw32-bsp README 首行链接、cw32-pacs README 生态图）。

## B.3 工具链仓

固件三层之外，还有三个工具向的仓库（出处：cw32-pacs README、cw32-bsp README
生态图）：

- **cw32-bsp**：严格说是"板级支持"而非纯工具——11 家族一套 crate，提供
  引脚复用全表（`pins` 模块，UART1/SPI1/I2C1 全部组合 + AF 编号，逐条带
  厂商 SDK 宏出处）、`bsp_pin!` 宏、blink/uart_echo 注入式模板
  （出处：cw32-bsp README）。
- **cw32-probe-rs-compat**：补齐 probe-rs 对 CW32 的两层缺失——target 层
  （9 个系列的 YAML，从官方 DFP pack 提取 Flash 算法转换）与自动识别层
  （`cw32-detect`，基于出厂区"数字签名"读型号串）。识别层已上游化：
  PR #4598 合并（2026-10-31），issue #4550 关闭，下一版 probe-rs 起
  `probe-rs info`/自动 attach 对 CW32 开箱即用；F030 target 的 PR #4549
  也已合并进上游（出处：cw32-probe-rs-compat README 顶部更新注——该表
  此行仍标"open"，属滞后口径；cw32-bsp README 生态图明确"#4549 F030 烧录
  + #4598 全系列自动识别，均已上游合并"）。详见附录 D。
- **cw32-tools**（VS Code 插件，仓内包名 `cw32l012-tools`，"CW32L012
  Tools"）：定位"构建/烧录/可视化调试/Agent API"，对标 Keil 的
  "编译→烧录→调试"工作流（构建、probe-rs 烧录/复位/全片擦除、经
  `probe-rs dap-server --single-session` 的 VS Code 原生断点调试）；
  内置只监听 127.0.0.1:3678 的 HTTP JSON Agent API（`/api/status`、
  `/api/build`、`/api/flash`、`/api/memory` 等），AI/脚本可直接 curl。
  版本口径：README/`package.json` 0.5.0（0.6.0 开发中，串口监视器并入
  面板、烧录后 verify 等）（出处：本地 checkout 的 README 与
  docs/AGENT_API.md；其 `api/memory` "仅在 IDE 调试会话开启时可用"的
  行为记录见 cw32l012-hal TESTING.md"通用操作"）。本书正文不依赖该插件。

## B.4 与厂商 C SDK / DFP 的关系

Rust 生态没有重新发明数据，而是从厂商资产出发逐层转换。对应关系：

| 厂商资产 | 在 Rust 生态中的用途 | 出处 |
|----------|----------------------|------|
| 《用户手册》/《数据手册》 | L012 PAC 的**手写转录唯一来源**（UM V1.4）；BSP 引脚 AF 表数据源（经 cw32-hal2 `cw32-data/af` 定位提取）；家族复位时钟以各家族数据手册原文为准 | cw32l012-pac README；cw32-bsp README；embassy-cw32 CHANGELOG |
| C 标准外设库（SDK 头文件） | `cw32-pac-gen` 的生成输入（10 家族 PAC）；经典组驱动配方的对照源（F030 `rcc` PLL 配方转录厂商库、经典组 ADC 按厂商库 V2.2 配方、I2C BRR 公式、DMA 触发源表各家手册 §8.8.4） | cw32-pacs README 首段；embassy-cw32 CHANGELOG 多条 |
| SVD | 上游 `cw32l012_pac` 0.1.1 的生成源；**本书生态不采用**（三处缺陷，见 B.2） | cw32l012-pac README |
| DFP pack（如 `WHXY.CW32L012_DFP` 1.0.2） | probe-rs target YAML 的生成源——Flash 算法（FLM）经 `target-gen`/等价脚本转成 YAML；compat 仓脚本链 `extract_all.py → cw32_gen.py` 从官方固件库 zip 复现同一流程 | cw32l012-hal README"Flash"段；cw32-probe-rs-compat README"目录结构/复现流程" |

转换质量有交叉校验记录（出处：cw32-probe-rs-compat README"验证情况"）：

- compat 仓生成的 `CW32F030_Series.yaml` 与 PR #4549（硬件已验证）的算法
  blob **逐字节一致**（632 B）；
- 同源流程产物的 L0 系列算法与上游已合并的 `CW32L0_Series.yaml` **逐字节
  一致**（412 B）；
- PAC 侧：手写 L012 PAC 有宿主回归测试（寄存器块 size/offset、基地址、
  32 项中断号三方一致）与实机 rt 冒烟（出处：cw32l012-pac README
  "Verification"）。

另外一条非本生态但值得一知的相关工作：fan2nd/cw32-hal2（2026 年 10 月，
早于 embassy-cw32）走 chiptool/metapac 全家族路线，驱动面很宽；embassy-cw32
选择"每家族一个 PAC"的路线，并从 cw32-hal2 吸收了 `BufferedUart` 设计
（MIT OR Apache-2.0，归属声明见其 `THIRD_PARTY.md`）（出处：embassy-cw32
README "Related work"、CHANGELOG 0.2.0）。

## B.5 版本演进时间线（关键节点）

- **2026-01-17**：probe-rs v0.31.0 内置 CW32L010/L011/L012 target
  （PR #3682）（出处：cw32-probe-rs-compat README 现状表）。
- **2026-06-25**：svd2rust 版 `cw32l012_pac` 0.1.1 上 crates.io（与本书
  生态无关的名字占位）（出处：cw32l012-pac README）。
- **2026-10-17 ~ 10-24**：cw32l012-hal 实机验证集中期（TESTING.md 记录
  2026-10-17 起的首批例程到 10-24 的 ecosystem 例程）；embassy-cw32
  0.1.0/0.2.0 发布（2026-10-24）；cw32l012-pac `rt` 冒烟同日
  （出处：TESTING.md；embassy-cw32 CHANGELOG；cw32l012-pac README）。
- **2026-10-31**：CW32 识别层上游化（PR #4598 合并、#4550 关闭）；
  cw32-detect 对 probe-rs 0.32.0 构建零警告、板上识别验证
  （出处：cw32-probe-rs-compat README 顶部注与"板上验证"）。F030 target
  PR #4549 已合并进上游（合并日期以 probe-rs 仓为准；出处：cw32-bsp
  README 生态图"均已上游合并"）。
- **待定**：cw32-pacs 的 crates.io 档期；`cw32l012-pac` 换名发布；
  embassy-cw32 0.4.0 定档（随 DMA/RTC 收尾）（出处：各仓 README/CHANGELOG）。
