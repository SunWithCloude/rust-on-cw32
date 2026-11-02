# 第 1 章 环境搭建

这一章你会学到：

- 用 `rustup` 装好 Rust stable 工具链，并加上单片机需要的交叉编译目标
  `thumbv6m-none-eabi`；
- 安装烧录与调试工具 probe-rs，认识你手里的调试器；
- 配好带 `rust-analyzer` 的编辑器；
- 理解本书仓库与三个生态仓库的目录布局约定，弄清 `path` 依赖为什么这么挂；
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

## 仓库布局：一本书带三个生态仓

本书的例程不把依赖打包在自己怀里，而是直接引用三个生态仓库的源码：

| 仓库 | 角色 | 本书引用方式 |
|------|------|--------------|
| `cw32l012-pac` | 寄存器访问层（PAC） | `examples/pac` 例程 crate |
| `cw32l012-hal` | 阻塞式 HAL（crate 在仓库内同名子目录） | `examples/hal` 例程 crate |
| `embassy-cw32` | 异步 HAL | `examples/embassy` 例程 crate |

约定它们与本书仓库**平级**放置（下例以 `book/` 代指你放代码的任意目录）：

```text
book/
├── rust-on-cw32/          # 本书仓库（本仓）
│   └── examples/pac/Cargo.toml
├── cw32l012-pac/          # PAC：仓库根就是 crate
├── cw32l012-hal/          # HAL：仓库根下套一层同名 crate 目录
│   └── cw32l012-hal/
└── embassy-cw32/          # 异步 HAL：仓库根就是 crate
```

克隆时照这个位置放即可：

```bash
cd book
git clone https://github.com/SunWithCloude/rust-on-cw32
git clone https://github.com/SunWithCloude/cw32l012-pac
git clone https://github.com/SunWithCloude/cw32l012-hal
git clone https://github.com/SunWithCloude/embassy-cw32
```

## path 依赖为什么这么挂

打开 `examples/pac/Cargo.toml`，依赖是这样写的：

```toml
[dependencies]
cw32l012-pac = { path = "../../../cw32l012-pac", features = ["rt"] }
```

`../../../` 从 `examples/pac/` 出发向上三级，正好落在 `book/`——也就是
三个生态仓的平级位置。Cargo 的 `path` 依赖按**相对当前 Cargo.toml 的路径**
解析，所以只要克隆时保持平级布局，一切直接可用，不需要额外安装任何
生态组件。

这样设计有两个理由：

- **改一行立刻见效**。本书大量讲解 PAC/HAL 的实现细节（比如第 2 章逐行拆
  KEY 写保护的写序列），你在生态仓源码里加一行注释、打个标记，重新编译
  例程就能观察变化——发布到 crates.io 的依赖做不到这一点。
- **版本永远跟着仓库走**。三个生态仓演进很快（HAL 还在快速补外设），
  path 依赖让"书的代码"与"仓库的代码"永远是同一份，不会出现"书上是
  v0.9 的 API、crates.io 上已经是 v1.0"的错位。

代价是克隆位置必须正确。如果你把生态仓放到了别处，编译会报
`failed to load source for dependency ... cw32l012-pac`——把目录挪回平级
位置即可，或者在例程 crate 的 `Cargo.toml` 里把 `path` 改成你的实际位置。

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
- **生态仓没放在平级位置**：报 `failed to load source for dependency`，
  按上文布局挪目录或改 `path`。
- **首次编译很慢**：是正常的——三个生态仓在为 M0+ 完整编译一遍，之后
  增量编译都是秒级。
- **WSL 里装了 rustup、Windows 里装了 probe-rs（或反过来）**：两套环境
  互看不见，选一边装齐。
