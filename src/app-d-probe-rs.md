# 附录 D probe-rs 速查

烧录、读内存、RTT 日志的命令卡片。命令行为随 probe-rs 版本有差异，
**版本口径以 cw32-probe-rs-compat 仓 README 为准**（本附录每节标出处）。

## D.1 指定芯片的两种方式

**方式一：`--chip <芯片名>`（L0 系列开箱可用）**

```sh
probe-rs download --chip CW32L012x8 firmware.elf    # probe-rs ≥ v0.31.0
```

CW32L010/L011/L012 的 target 定义自 **v0.31.0**（2026-01-17）起内置
（PR #3682）（出处：cw32-probe-rs-compat README 现状表）。

**方式二：`--chip-description-path <yaml>`（其它系列 / 旧版 probe-rs）**

```sh
# compat 仓 targets/ 下 9 个系列的本地 target 文件
probe-rs download --chip CW32F030C8 \
    --chip-description-path targets/CW32F030_Series.yaml firmware.elf
```

`cargo-embed` / `cargo-flash` 同样支持 `--chip-description-path`；VS Code
probe-rs 调试器用 `launch.json` 的 `chipDescriptionPath` 字段
（出处：cw32-probe-rs-compat README "快速开始 B"）。cw32l012-hal 仓的
`tools/CW32L0_Series.yaml` 是同一流程（`target-gen` 从厂商 DFP 1.0.2 生成）
的产物，供 probe-rs < 0.31 的场景使用（出处：cw32l012-hal README
"Debugging in VS Code"）。

常用变体名（与 `probe-rs chip list` 风格一致，来自官方 pdsc 器件表；
出处：cw32-probe-rs-compat README 变体表）：

| 系列 | 变体 |
|------|------|
| CW32L012 | `CW32L012x8`（内置） |
| CW32F030 | CW32F030C8 / K8 / F8（64K）、CW32F030F6（32K） |
| CW32F003 | CW32F003F4（20K） |
| CW32L031 | CW32L031C8 / K8 / F8（64K） |
| CW32L083 | CW32L083VC / MC / RC（256K）、CW32L083RB（128K） |
| CW32W031 | CW32W031R8（64K） |

（完整 9 系列清单见 compat 仓 README。）

## D.2 常用命令

以下 `$PROBE` / `$CFG` 沿用 cw32l012-hal TESTING.md 的口径：
`--probe 0483:3748 --protocol swd --chip CW32L012x8` 与
`--chip-description-path <CW32L0_Series.yaml>`（新版 probe-rs 可省 CFG）
（出处：TESTING.md"通用操作"）。

```sh
# 烧录（download = 写 Flash；无需先 erase）
probe-rs download $PROBE $CFG target/thumbv6m-none-eabi/release/examples/blink

# 复位（让新固件跑起来）
probe-rs reset $PROBE $CFG

# 全片擦除
probe-rs erase --chip CW32L012x8 --allow-erase-all
#   （出处：cw32l012-vscode README 功能总表"全片擦除"行）

# 读内存：SRAM 快照首选直读
probe-rs read b8 0x20000000 128 $PROBE $CFG     # 读 128 字节，带地址前缀 hexdump
#   b16/b32 同型，按 16/32 位单位读取，适合直接看 u16/u32 数组；
#   各宽度的计数单位以 probe-rs read --help 为准（出处：TESTING.md
#   "观察静态变量"，b8=字节口径为实测原样）

# 烧录 + 运行 + RTT 日志一条龙（见 D.4）
probe-rs run $PROBE $CFG target/thumbv6m-none-eabi/release/examples/ch03_rtt
```

**读 SRAM 的完整流程**（观察观察变量/静态量）：

1. `nm <elf> | grep -E " [bBdD] "` 定位符号地址——**每次重编译都会漂移，
   必须重新查**（出处：TESTING.md"通用操作"第 1 条）；
2. `probe-rs read b8 <符号地址> <字节数>`；hexdump 带地址前缀，解析时按行
   `split(':')` 后按字节顺序拼（出处：TESTING.md `ts_probe` 节）；
3. `probe-rs read` 是首选——直连探针，不依赖 IDE；插件调试服务
   `http://127.0.0.1:3678/api/memory` 仅在 IDE 调试会话开启时可用
   （出处：TESTING.md"通用操作"第 2 条）；
4. ⚠ 外设寄存器（APB）**不要**用调试器直读——实测出现过三个寄存器同值
   0x89D 的假读；要寄存器值，固件里 `read_volatile` 快照到 SRAM 再读
   （出处：TESTING.md"通用操作"第 3 条）。

**cargo run 直通**：cw32l012-hal 仓的 `.cargo/config.toml` 已把上面两组
参数接进 `probe-rs run` runner，`cargo run --target thumbv6m-none-eabi
--release --example blink` 即烧即跑（出处：cw32l012-hal README "Flash"节）。

## D.3 芯片识别（新版开箱识别 / 旧版的行为）

| probe-rs 版本 | `--chip CW32L012x8` | `probe-rs info` 自动识别 |
|--------------|---------------------|--------------------------|
| < 0.31 | 不识别 → 需 `--chip-description-path` | 不可用 |
| 0.31 / 0.32 | **开箱可用**（L0 系列内置） | 失败：`could not automatically be determined` |
| 0.33+（识别层上游化后） | 开箱可用 | **开箱识别 CW32 全系** |

- 0.32 上自动识别失败的原因：CW32 全系 MINDP（DPv1 无 SWD multidrop），
  标准 ARM 调试拓扑里没有任何厂商唯一信息，ROM 表只到 ARM 通用组件就停
  （`--verbose` 可见 "Debug port version DPv1 does not support SWD
  multidrop"）（出处：cw32l012-pac docs/probe-rs-feedback/
  issue-auto-detection.md；cw32-probe-rs-compat README"背景"节）。
- 识别层的解法是读**出厂区数字签名**（型号串/Flash/RAM/管脚数/UID），
  已随 PR #4598 合并进上游（2026-10-31，issue #4550 关闭）；F030 target
  的 PR #4549 亦已合并。下一版 probe-rs 起 `probe-rs info` / 不带
  `--chip` 的自动 attach 对 CW32 开箱即用（出处：cw32-probe-rs-compat
  README 顶部更新注；cw32-bsp README 生态图"均已上游合并"）。
- 上游版本号口径：CONVENTIONS §5 记为"上游 0.33+ 开箱可识别"；确切
  版本以 probe-rs 发布为准。

**独立识别工具 `cw32-detect`**（compat 仓交付，不升级 probe-rs 也能用）：

```sh
cd cw32-detect && cargo build --release
./target/release/cw32-detect          # 人类可读输出
cw32-detect --json                    # JSON（脚本集成）
cw32-detect --probe 0483:3748         # 指定探针 VID:PID[:序列号]

# 识别 + 烧录一步组合
probe-rs download --chip $(./target/release/cw32-detect -q) \
    --chip-description-path targets/CW32F030_Series.yaml firmware.elf
```

退出码：0 识别成功 / 1 未发现 CW32 / 2 探针错误。识别覆盖全部 12 个系列；
L0 家族识别后直接输出内置变体名（如 `CW32L012x8`），无需 target 文件。
厂商型号串带封装后缀（实测输出 `CW32L012C8T6`），工具按 x 归一化匹配到
`CW32L012x8`（匹配方式 NormalizedX）（出处：cw32-probe-rs-compat README
"快速开始 C"与"板上验证"节，2026-10-31 实板）。

## D.4 RTT 日志

```sh
probe-rs run --chip CW32L012x8 <elf>   # 烧录 + 复位 + 自动附加 RTT
```

- `probe-rs run` 会在固件运行时把 RTT 接收到的日志打到终端；hal 仓已把
  它接成 `cargo run` 的 runner（出处：cw32l012-hal README）。
- 固件侧两件套：`defmt-rtt`（日志编码走 RTT 通道）+ `panic-probe`
  （panic 时打印并 halt；`features = ["print-defmt"]` 时打 defmt 格式）。
  本书 `examples/pac`、`examples/embassy` 的 dev-dependencies 即此组合；
  embassy-cw32 的 `rtt_hello` 示例是端到端参照（defmt 时间戳用
  embassy-time uptime 毫秒手写 `_defmt_timestamp`，不引额外 crate）
  （出处：embassy-cw32 CHANGELOG `rtt_hello` 条；本书 examples/*/Cargo.toml）。
- 收不到日志时先查两件事：RTT 控制块是否被编译器保留（`#[used]` /
  `no_mangle`，见附录 C 条目 3）；probe-rs 是否用了与固件一致的
  `--chip`（错误 target 会影响 RAM 布局扫描）。

## D.5 常见错误表

| 错误 | 一句话处置 |
|------|-----------|
| `NoSupportedProbe` | 没找到探针：换 USB 口/装驱动，多探针时用 `--probe <VID:PID[:序列号]>` 指定（TESTING.md 的 `--probe 0483:3748` 口径） |
| `JtagGetIdcodeError` | 协议回退到 JTAG 了：加 `--protocol swd`，VS Code 里 `wireProtocol: "Swd"`（实测必踩，出处：cw32l012-hal README "Two things that cost us time"） |
| `SwdApWdataError` / 其它 `SwdAp*`（AP sticky error） | 目标板断电数秒重插；不行 `probe-rs reset` 拿回 AP——详见附录 C 条目 1 |
| `The connected chip could not automatically be determined` | 0.31/0.32 没有识别层，属预期：显式给 `--chip`（出处：cw32l012-pac docs probe-rs-feedback 实录） |
| `Unable to load specification for chip` / 芯片不在列表 | 用 `--chip-description-path` 给本地 yaml，或升级 probe-rs（出处：compat README 快速开始 B） |
| `WaitForDapPinsStuck` / DAP 握手卡住 | 目标没在正常跑（掉电/停在复位/固件睡了）：查供电与 RESET 线，试 `--connect-under-reset`；固件侧避免 `wfi` 收尾（出处：probe-rs 通用错误；wfi 案例见 TESTING.md `ts_probe3`） |
| 读保护芯片报"未识别" | 数字签名区可能不可读——先解保护再识别（出处：compat README"已知限制"） |
| 连接后闪一下就断 | 目标固件把调试口配掉或进了低功耗：复位后立即 attach，或查固件的休眠/引脚配置（出处：compat README MINDP/sticky 机制） |

错误名出处分层：`JtagGetIdcodeError` / `SwdApWdataError` / "could not
automatically be determined" 有实机记录（表中已标）；`NoSupportedProbe` /
`WaitForDapPinsStuck` 是 probe-rs 通用错误名（非 CW32 特有，本地素材仓
未载，措辞随版本可能不同），以你的 probe-rs 输出为准。

## D.6 GDB server 与 VS Code 调试注意点

- **别用 Cortex-Debug**：它走 GDB，需要 `arm-none-eabi-gdb` + GDB server
  （OpenOCD / ST-Link GDB server / J-Link），而**没有任何一家 GDB server
  带 CW32L0 的 target 定义**。probe-rs VS Code 扩展直接说 DAP，不需要
  GDB（出处：cw32l012-hal README "Debugging in VS Code"）。
- VS Code 路线：`probe-rs.probe-rs-debugger` 扩展 + `wireProtocol: "Swd"`
  + `flashingConfig.flashingEnabled`；probe-rs 0.32 起 `chip` 直接填
  `CW32L012x8`（`probe-rs chip list` 可见 CW32L0 Series），< 0.32 才需要
  `chipDescriptionPath`（出处：cw32l012-hal README）。
- 本仓插件的调试后端是 `probe-rs dap-server --single-session`（出处：
  cw32l012-vscode README 功能总表）。
- **内存映射与 PPB 访问**：有说法称 target yaml 的内存映射可能使调试器
  拒绝对 PPB 段（0xE000_0000 起，SysTick/NVIC/SCB）的访问——该条**未
  在 cw32-probe-rs-compat 仓现行 README 中检索到，未核实**；遇到"读不到
  内核外设"时以 compat 仓最新记录与 probe-rs 该版本源码为准。有实机记录
  的近似问题是 APB 外设寄存器假读（处置见 D.2 第 4 条，出处：TESTING.md）。

## D.7 版本行为差异

本附录命令示例横跨 probe-rs 0.31/0.32 与上游化前后的行为差异：
`--chip` 开箱范围、自动识别、`read` 各宽度计数语义等都可能随版本变化。
**排查命令不生效时，先对版本，再对命令**——以 cw32-probe-rs-compat 仓
README（及其"已知限制"节）为口径基准；compat 仓自身依赖 probe-rs
0.32.x 公开 API（出处：compat README"已知限制"）。
