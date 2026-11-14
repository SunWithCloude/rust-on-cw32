# 第 1 章 环境搭建

这一章你会学到：

- 用 `rustup` 装好 Rust stable 工具链，并加上单片机需要的交叉编译目标
  `thumbv6m-none-eabi`；
- 安装烧录与调试工具 probe-rs，认识你手里的调试器；
- 配好带 `rust-analyzer` 的编辑器；
- 认识三个生态依赖怎么声明——crates.io 精确钉版（三个依赖全走版本引用），
  **跑通本书不需要克隆任何仓库**；
- 用一条命令验证整个环境：把书里全部例程编译成 Cortex-M0+ 固件。

本章没有固件例程——目标是结束时，第 2 章的 `ch02_blink` 已经能一键编译。

## 安装 Rust

Rust 由 [`rustup`](https://rustup.rs) 安装管理，三平台入口一致：

```bash
# Windows：下载并运行 rustup-init.exe（https://rustup.rs 首页链接）
# Linux / macOS：
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Windows 上 `rustup-init.exe` 一路回车即可；提示缺 Visual Studio Build Tools
时按它的指引装一遍（MSVC 目标需要它做链接器）。

装完重开一个终端，确认版本：

```bash
rustc --version
```

本书写作时用的是 1.97 系 stable。**不必追最新**，但请用 stable 通道——
本书所有例程只依赖 stable 特性，Rust 2024 edition 需要 1.85 及以上。

## 加上交叉编译目标

`rustup` 默认只装"编译给本机"的工具链。单片机是 ARM Cortex-M0+ 内核
（ARMv6-M 架构），需要一个单独的编译目标：

```bash
rustup target add thumbv6m-none-eabi
```

这条命令往工具链里放一份 `thumbv6m-none-eabi` 的核心库（`core` 等的预编译
副本）。名字怎么读：`thumb` 是 ARM 的精简指令集状态，`v6m` 对应 ARMv6-M
（Cortex-M0/M0+），`none-eabi` 表示"没有操作系统、嵌入式 ABI"。本书全程
只为这一个目标编译。

## 安装 probe-rs

[probe-rs](https://probe.rs) 是本书的烧录与调试后端：烧写固件、复位芯片、
收 RTT 日志（第 3 章）、断点调试都靠它。它直接支持 ST-Link、DAP-Link、
J-Link 等常见调试器。

安装方式按平台选一种（完整说明见
[官方安装文档](https://probe.rs/docs/getting-started/installation/)）：

```bash
# 全平台通用：从源码编译安装（约几分钟，务必带 --locked）
cargo install probe-rs-tools --locked

# Windows：到 https://github.com/probe-rs/probe-rs/releases 下载安装包运行
# macOS / Linux：brew install probe-rs-tools
# 已装 cargo-binstall 的话：cargo binstall probe-rs-tools
```

CW32L012 的支持（目标芯片名 `CW32L012x8`）在 probe-rs **0.33 及以上**
开箱可用，装完顺手确认：

```bash
probe-rs --version
```

把调试器接上开发板（SWD 四根线：SWDIO、SWCLK、GND，通常再加 3V3 供电），
插到电脑上，然后：

```bash
probe-rs list
```

输出里列出你的探针（ST-Link V2 显示为 `0483:3748`）就说明 USB 这条路通了。
找不到探针时的排查思路见附录 D。

## 编辑器

任何编辑器都能写 Rust，但 **rust-analyzer 必装**——它不是补全插件那么简单，
没有它你就没有跳转、没有类型提示、没有寄存器名补全，本书的 PAC 裸寄存器
代码会写得非常痛苦。

- **VS Code**（推荐）：装 `rust-analyzer` 官方扩展即可，无需额外配置。
- 其他编辑器：rust-analyzer 以 LSP 服务器的形式存在，Vim/Emacs/Neovim 各有
  成熟接入方式，见 [rust-analyzer 用户手册](https://rust-analyzer.github.io/manual.html)。

可选：VS Code 的 **probe-rs debugging** 扩展提供图形化断点调试，烧录、
断点、看变量都在编辑器里完成。第 2、3 章只用命令行就够，这个扩展到第 7 章
以后再装也不迟。

## 生态依赖：三行声明，不需要克隆任何仓库

本书例程的三个生态依赖（第 4 章起逐个登场），来源与版本口径如下：

| 依赖 | 角色 | 版本与引入方式 |
|------|------|----------------|
| `cw32l012-pac-um` | 寄存器访问层（PAC） | crates.io，精确钉版 `=0.3.0` |
| `cw32l012-hal` | 阻塞式 HAL（crate 在仓库内同名子目录） | crates.io，`"0.9.2"` |
| `embassy-cw32` | 异步 HAL | crates.io，精确钉版 `=0.4.0` |

**结论先行：跑通本书不需要克隆任何生态仓库。** 三个依赖全部由 cargo
自动拉取。装好 Rust 后，你在任何目录都可以这样起步自己的工程：

```bash
cargo new my-first-cw32
cd my-first-cw32
cargo add cw32l012-pac-um
```

`cargo add cw32l012-pac-um` 就把寄存器层装进了 `Cargo.toml`——PAC 是
零依赖的纯寄存器层，一条命令即用。（名字里的 `-um` 后缀：crates.io 上
`cw32l012_pac` 已被第三方 svd2rust 缺陷版占用，手写转录版改名发布，
故事见附录 B。）

三个例程 crate（`examples/pac`、`examples/hal`、`examples/embassy`）
的依赖声明就是这三行（第 2 章起逐个拆解）：

```toml
# examples/pac/Cargo.toml
cw32l012-pac-um = { version = "=0.3.0", features = ["rt"] }

# examples/hal/Cargo.toml
cw32l012-hal = "0.9.2"

# examples/embassy/Cargo.toml
embassy-cw32 = { git = "https://github.com/SunWithCloude/Cw32l012_embassy", branch = "main", features = ["rt"] }
```

两个口径说明：

- **为什么 PAC 用 `=` 精确钉版**：寄存器层的任何"自动升级"都可能悄悄
  改变位段假设（位面错一位，整个固件的行为全变），升级必须手动改版本
  号并重跑全部例程。HAL 用正常 semver（`0.9.2`）即可。embassy-cw32 0.4.0
  起已发布 crates.io，同样精确钉版（本书用 `=0.4.0`）。
- **想改生态仓源码的开发者**：把生态仓克隆到任意位置，在**自己工程根的
  `Cargo.toml`** 加一段 `[patch.crates-io]`，把 crates.io 依赖指回本地
  检出（本书写作期就是用这个办法边改边验证的）：

```toml
[patch.crates-io]
cw32l012-pac-um = { path = "../cw32l012-pac" }
cw32l012-hal = { path = "../cw32l012-hal/cw32l012-hal" }
embassy-cw32 = { path = "../embassy-cw32" }
```

改一行源码、重新编译例程，立刻见效——这是只读的版本依赖给不了的
工作流，但它属于进阶玩法，书里的例程一概不依赖它。

## 验收：一次编译全部例程

在**本书仓库根目录**执行：

```bash
# bash（Linux/macOS/Git Bash）
cargo build --workspace --target thumbv6m-none-eabi --release --examples
```

这行命令没有路径参数，PowerShell 下逐字相同，直接复制即可（提示符不同而已）：

```text
PS book\rust-on-cw32> cargo build --workspace --target thumbv6m-none-eabi --release --examples
```

命令含义逐段拆开：

- `--workspace`：编译仓库里全部三个例程 crate（`examples/pac`、
  `examples/hal`、`examples/embassy`）；
- `--target thumbv6m-none-eabi`：为 Cortex-M0+ 交叉编译，而不是本机；
- `--release`：优化构建（固件体积与速度都按发布标准来）；
- `--examples`：编译每个 crate 的 `examples/` 目录下的例程文件。

首次会连生态仓一起编译，几分钟量级；结束时逐一 `Finished` 而没有 error，
代表环境完全就绪。想单独编译某一章的例程（本书各章给出的都是这种形式），
加 `-p` 限定 crate：

```bash
cargo build --release --target thumbv6m-none-eabi --example ch02_blink -p cw32-book-pac
```

> 想把本书本身构建成网页阅读（可选）：`cargo install mdbook` 后在仓库根
> 执行 `mdbook build`。

## 常见坑

- **`rustup target add` 忘了跑**：报错 `can't find crate for 'core'`——
  目标的核心库没装，回第 2 节补上即可。
- **`cargo install probe-rs-tools` 不带 `--locked`**：依赖解析可能失败，
  官方明确要求带 `--locked`。
- **依赖写成 `cw32l012-pac`（漏了 `-um`）**：cargo 会拿到 crates.io 上
  同名的第三方 svd2rust 缺陷版（API 对不上，三处寄存器缺陷）——本生态
  的手写 PAC 名叫 `cw32l012-pac-um`，见附录 B 的命名故事。
- **首次构建卡在拉取 crates.io 索引**：三个生态依赖首次解析要联网
  （仅 crates.io，不 clone 任何 GitHub 仓），离线环境请先在有网的机器上
  `cargo fetch`。
- **首次编译很慢**：是正常的——三个生态依赖在为 M0+ 完整编译一遍，之后
  增量编译都是秒级。
- **WSL 里装了 rustup、Windows 里装了 probe-rs（或反过来）**：两套环境
  互看不见，选一边装齐。
