# 附录 C 故障排查手册

每个条目都是"症状 → 原因 → 处置"三段式，处置步骤可以直接照做。
出处统一写在条目末尾；标注"实测"的都有实板记录（TESTING.md /
embassy-cw32 CHANGELOG / 实机验证注记）。

## C.1 症状速查表

| 症状 | 最可能原因 | 处置 | 条目 |
|------|-----------|------|------|
| probe-rs 报 `SwdApWdataError` 等 SwdAp* 错误，或烧完一次后连不上 | MINDP 调试端口的 sticky error 未清 | 目标板**完全断电数秒再重插**；不行再 `probe-rs reset` 拿回 AP | 1 |
| blink 慢固定整数倍（如 500ms 变 3s/6s），所有延时同比例偏慢 | 代码假设的时钟频率 ≠ 复位默认（L012 代 4 MHz / 经典代 8 MHz） | 按复位默认重算；变频后用实际频率重新声明 | 2 |
| `nm` 找不到观察用 static；或读数像"别的变量" | 只写不读的 static 被优化删除；重编译后地址漂移 | `write_volatile` 写观察量；`#[used]` 锁符号；重新 `nm` 定位 | 3 |
| 链接报 critical-section 重复/缺失 | 一份固件只允许一个 critical-section 实现 | hal 与 embassy-cw32 的实现**二选一**；本书用独立例程 crate 隔离 | 4 |
| `no matching package named ... found` | 声明的 path 依赖路径上没有 manifest | 按 §C.2 条目 5 的布局落仓，确认目录里有 `Cargo.toml` | 5 |
| LED 不亮 | 模拟态 → 门控 → 极性，按序排查 | 见条目 6 的三步序 | 6 |
| UART/SPI 无波形，寄存器配置看起来全对 | 引脚 AF 编号配错（家族间不同） | 查 cw32-bsp 对应家族 `pins` 模块全表 | 7 |
| VDDA/BGR 正常，温度通道恒 ≈0 counts，`temperature_mc()` 报错 | 硅片个体状态：该芯片 TS 模拟块无输出 | 跑 `ts_probe` 建基线；换芯片一锤定音；软件侧无解 | 8 |
| 调试器直读外设寄存器出现"同值假读" | 调试器 APB 直读不可信（实测假读） | 固件里 `read_volatile` 快照到 SRAM 静态变量再读 | C.3 |

## C.2 展开条目

### 条目 1：SWD 楔死（`SwdApWdataError` / sticky error）

**症状**：`probe-rs` 连接或烧录时报 `SwdApWdataError`（或其它 `SwdAp*` /
AP 访问错误）；更隐蔽的形态是"烧录成功一次之后，再次 attach 就连不上"。

**原因**：CW32 全系列是 MINDP（DPv1，无 SWD multidrop）。AP 进入错误态后
sticky 位不会自动清——compat 仓的识别器为此专门设计"每个候选基址用全新
attach 周期"，并实测确认 `reinitialize()` 可清 MINDP sticky error
（出处：cw32-probe-rs-compat README"工作原理"/"自动识别链验证"）。
固件侧也会触发：例程末尾用 `wfi` 收尾后，首次读取连续报
`JtagGetIdcodeError`，连 `--connect-under-reset` 也超时（实测，出处：
cw32l012-hal TESTING.md `ts_probe3`"坑"）。

**处置（按序）**：

1. **目标板完全断电数秒，重新插上**——第一动作，多数情况就此恢复
   （出处：CONVENTIONS §5"调试中 SWD 楔死（SwdApWdataError）先给目标板
   断电重插"）；
2. 仍不行：`probe-rs reset --chip CW32L012x8`，实测靠一次 reset 重新
   拿回 AP（出处：TESTING.md `ts_probe3`）；
3. 板卡没有外引 RESET 线、以上都无效时：换一块板重验，排除板级个体问题；
4. **预防**：固件末尾不要用 `wfi` 收尾，改 `loop { core::hint::spin_loop(); }`
   ——数据是边跑边写进 SRAM 的，空转时随时可读（出处：TESTING.md
   `ts_probe3` 的建议）。

### 条目 2：时钟倍频错（blink 慢 N 倍）

**症状**：blink 周期是预期值的固定整数倍；RTT 时间戳、所有延时同比例偏慢。

**原因**：代码按错误的复位默认频率算时间。复位默认是硬事实：**L012 代
（L010/L011/L012）= 4 MHz（HSI 96M/24），经典代 8 家族 = 8 MHz（HSI
48M/6）**（出处：CONVENTIONS §5；embassy-cw32 README "Clock convention"）。
实板案例：F030 按 48 MHz 假设配置 SysTick，结果 SysTick 慢 6 倍、blink
周期 6 秒；上板排障发现，DFP 的 `Dclock=8000000` 佐证 8 MHz（实测，出处：
embassy-cw32 CHANGELOG 修复条）。

**处置**：

1. blink 异常时先算"慢了几倍"：6 倍 → 典型的 48M/8M 假设错；
2. embassy 侧：不改时钟树就保持 `Config::default()`（已按家族声明 4/8 MHz）；
   改了时钟树，先用寄存器层切换，再把**实际频率**传给 `init`
   （出处：embassy-cw32 README "Clock convention"）；
3. 阻塞侧：`Rcc` 变频会自动重配 flash 等待周期，`rcc.clocks()` 拿到的
   就是当前频率（出处：cw32l012-hal README rcc 节）；
4. 一切"用延时当时间基准"的测量，先确认延时精度——实测 `delay_us` 曾
   偏慢 +310%，修复后 +0.8%（出处：TESTING.md `clk_check`）。

### 条目 3：debug/观察符号被优化掉

**症状**：`nm <elf>` 找不到观察用 static；或读到的值明显"串位"
（实测曾把 `DS_WAKES` 读成 `LSI_HZ=15`；把地址漂移后的 `TS_RAW` 误当
`TEMP_MC` 读出 -271℃ 假象）。

**原因**：开了 `opt-level` 后，只写不读的 `static mut` 会被整段删除；
每次重编译后静态变量地址都会漂移（出处：TESTING.md"静态量被优化掉"
"静态变量地址漂移"两条）。

**处置**：

1. 观察量一律用 `write_volatile(addr_of_mut!(X), v)` 写——volatile 写
   有副作用，编译器不敢删（出处：TESTING.md，实测有效）；
2. 必须留在符号表里的裸函数/static：edition 2024 下用 `#[used]` +
   `#[unsafe(no_mangle)]`（出处：CONVENTIONS §3.4）；
3. **每次重编译都重新定位**：`nm <elf> | grep -E " [bBdD] "`，别复用
   上一次的地址（出处：TESTING.md"通用操作"第 1 条）。

### 条目 4：critical-section 实现冲突（本书例程分三个 crate 的原因）

**症状**：链接失败，报 critical-section 实现重复/缺失类错误
（如 `no critical section implementation`，或两套实现同时启用时的
符号/链接冲突）。

**原因**：`critical-section` crate 的约定是**一个固件只能有一个实现**。
embassy-cw32 的实现由 cortex-m 的 `critical-section-single-core` feature
随依赖自动提供（embassy 自己的 `critical-section-impl` feature 已在
0.4.0 删除）；`cw32l012-hal` 则带自己的 `critical-section-impl` feature。
同一固件里两套实现同时进链接就会失败（出处：embassy-cw32 README
"Relationship to cw32l012-hal" ⚠ 注与 critical-section 行；
cw32l012-hal README "Only one critical-section implementation may exist
per binary"）。

**处置**：

1. 二选一：embassy 侧用 cortex-m 的实现（随依赖默认带上），hal 侧用
   自己的 `critical-section-impl`——两者不出现在同一份固件里；
2. **本书的做法**：例程拆成三个独立 crate——`examples/pac`、
   `examples/hal`、`examples/embassy`，阻塞线与异步线永远不进同一份
   固件（出处：本书根 `Cargo.toml` members；CONVENTIONS §5 已知坑）；
3. `examples/pac` 不依赖任何一方的实现，按第 3 章手写 PRIMASK 版
   （与 cortex-m 的 single-core 实现同一写法）。

### 条目 5：依赖解析失败（`no matching package found`）

**症状**：`cargo build` 报 `no matching package named ... found`，或
git 依赖更新卡住/失败。

**原因与处置**（生态依赖已全部走 crates.io 版本依赖 + 一个 git 依赖，
原"平级克隆 + path 依赖"工作流已废弃）：

1. **crate 名写错**：手写 PAC 的 crates.io 正式名是
   `cw32l012-pac-um`——写成 `cw32l012-pac` 拉到的是第三方 svd2rust
   缺陷版；F030/L010 同理，正式名是 `cw32f030-pac-um`/`cw32l010-pac-um`
   （出处：附录 B 的三个名字陷阱）；
2. **git 依赖（embassy-cw32）更新失败**：确认网络可达 github.com；
   代理环境给 cargo 配 `CARGO_HTTP_PROXY`；离线机器先在有网机器上
   `cargo fetch` 再拷贝 cargo 缓存；
3. **开发者用 `[patch.crates-io]` 指本地检出时**：patch 的 `path` 必须
   指向**含 Cargo.toml 的 crate 目录**（HAL 是仓库根下再套一层
   `cw32l012-hal/`），且本地 crate 的 `version` 要与被 patch 的
   crates.io 版本一致——Cargo 按版本匹配 patch，对不上则静默不生效。

### 条目 6：LED 不亮排查序（模拟态 → 门控 → 极性）

**处置（严格按序，前一步通过才查下一步）**：

1. **模拟态**：复位默认 GPIO 处于 ANALOG——L012 的 PC13 是
   `GPIOx_ANALOG=1`（出处：CONVENTIONS §5）；F030 更狠，全端口复位值
   `0x9fff`、仅 SWD 两根线数字使能（实测，出处：embassy-cw32 CHANGELOG
   I2C 多家族化条）。点灯前必须清模拟位/切数字功能；embassy 驱动的
   `Output::new` 已处理，裸寄存器要自己清。
2. **门控**：AHBEN 里对应 GPIO 端口的位是否置上。L012 代是 **KEY =
   0x5A5A 带键写**——漏 KEY 的写会被静默丢弃（出处：cw32l012-pac
   README 用法示例；embassy-cw32 CHANGELOG"门控写规范"条）；经典代
   8 家族无键、普通读改写。"静默丢弃"是这类坑的共同面目——Flash 自编程
   漏开 `AHBEN.FLASH(bit1)` 时擦写同样变成空操作且不报错（实测，出处：
   TESTING.md `flash_store` 坑 1）。
3. **极性**：LED 另一端接 VCC 的板子（含本书用户板的 PC13 典型接法），
   输出**低**电平点亮——`Level::Low` / 清 ODR；接 GND 则相反。拿不准就
   先 toggle 再看万用表/示波器。
4. 以上都对还不亮：查原理图换引脚——用户板 PC13、F030 实板 PC13，
   generic 模板从 PA0 起步（出处：cw32-bsp README 实板支持列）。
5. 模拟外设引脚是同一类坑的反向版：PB00 必须 `into_analog()`，否则
   数字输出驱动把节点压在 0V，OPA 输出被短路（实测，出处：TESTING.md
   `analog_loop` 坑 2）。

### 条目 7：引脚 AF 配错

**症状**：UART/SPI/I2C 怎么调都无波形；寄存器链逐项检查又都"正确"。

**原因**：AF 编号与可用引脚组合**随家族不同**——L012 的 UART1 =
PA02(TX)/PA03(RX) **AF1**（实测，出处：CONVENTIONS §5）；F030 的
UART1 = PA08/PA09 **AF2**（实板验证，出处：cw32-bsp README 实板支持
列）。照抄另一家族的引脚号/AF 编号必错。

**处置**：

1. 打开 cw32-bsp 对应家族的 `pins` 模块：UART1/SPI1/I2C1 的全部
   `(端口, 引脚号, AF 编号)` 组合都在表里，逐条带厂商 SDK 宏出处
   （出处：cw32-bsp README）；
2. 复用输入类信号（如 ATIM 刹车输入）必须配成 **AF 复用输入**——实测
   PA06 刹车脚用普通 GPIO 输入/悬空时 BRK 采样为低 → 刹车锁存 →
   `BDTR.MOE` 被硬件清零 → 引脚无波形，改 `into_af_input(Af::Af7)`
   后恢复（实测，出处：TESTING.md `atim_deadtime` 坑）；
3. L012 例程移植到其它家族时，引脚/AF 一律重新查表，见附录 A.3 第 4 步。

### 条目 8：温度传感器"无输出"是个体差异，不是代码错

**症状**：VDDA 与 1.2V BGR 通道读数正常，TS 通道（内部温度传感器）恒为
0~7 counts（≈0V）；`temperature_mc()` 返回 `AdcError::SensorDead`。

**原因**：**硅片个体状态**——该芯片 TS 模拟块（偏置/输出级）无输出。
软件侧手段已全部穷尽：使能顺序 5 变体、稳定期、BGR 软件重启、ADC2、
采样时间 S6~S518+Div8、90s+ 预热、受控 POR（断电 10s 重插后 BGR 恢复
理论值 1492，TS 仍为 0）；第三轮 `ts_probe3` 六个新角度全灭——最关键
的对照是 **TSEN=0 与 TSEN=1 读数完全相同**，说明偏置电流根本没到
传感器；放电曲线三点拟合 τ≈1.03µs 一致，节点呈"0V + 约 100~250kΩ
泄放通路"特征（实测，出处：cw32l012-hal TESTING.md TS 条目
`ts_probe` 判定矩阵与 `ts_probe3` 六角度扫描、POR 终审）。

**处置**：

1. 先跑 `ts_probe`（一次烧录 30s 跑完 A/B/C/曲线/S6 五相位）建基线，
   排除自己代码的问题（出处：TESTING.md `ts_probe` 节）；
2. 基线与已知个体异常一致（TS 全 0、BGR ≈cal×4095/VDDA ±2%）→ 换一颗
   CW32L012 跑同一份固件一锤定音（出处：TESTING.md"同一份固件换一颗
   即可一锤定音"）；
3. 软件侧无解，**不要再调参**。库已按防御处理：`temperature_mc*()` 在
   raw < 100 counts 时返回 `AdcError::SensorDead`，杜绝"-271℃"假数据
   （出处：cw32l012-hal README Known limitations"dead TS node on the
   die"、src/adc.rs `AdcError::SensorDead`；embassy-cw32 CHANGELOG
   0.4.0 段同口径收敛进错误路径）。

## C.3 寄存器级速查（小坑集中营）

| 症状 | 原因 | 处置 |
|------|------|------|
| ICR 写 1 标志不清 | DMA/UART/GTIM 的 ICR 是 **R1W0**：写 1 无动作，写 **0** 才清 | `icr.write(!(1<<0 \| 1<<1))` 这类"取反写"（出处：TESTING.md `dma_demo` 坑 1） |
| DMA 每轮只搬一次 | `CNT.REPEAT` 每次传输后被硬件清 0 | 每轮传输前重新 `configure()`（出处：TESTING.md `dma_demo` 坑 2） |
| Flash 擦写"看起来成功"实则没发生 | `AHBEN.FLASH(bit1)` 未开，或 MODE 写成裸 `KEY \| MODE` | `Flash::new` 自动开门控；MODE 必须**或进现有 CR1**（出处：TESTING.md `flash_store` 两个坑） |
| LPTIM/LSI 卡死不跑 | LSI 复位默认**关闭** | `Rcc::enable_lsi()`（读 TRIM → `CR1.LSIEN` → 轮询 STABLE）（出处：TESTING.md `lptim_deepsleep` 坑 1） |
| OPA 配置读回恒为复位值 | 门控在 **APBEN2[9]**，不是 APBEN1 | 使能正确的门控位（出处：TESTING.md `analog_loop` 坑 1） |
| 调试器直读外设寄存器得到"同值假读" | 实测 SAMPLE/SQRCFR/RESULT1 三寄存器同值 0x89D | 固件 `read_volatile` 快照到 SRAM 再读（出处：TESTING.md"通用操作"第 3 条） |
| RTC 停表状态下写时间一直超时 | "等 WAIT"流程只适用于**运行中**写 DATE/TIME | 停表后直接写（出处：TESTING.md `rtc_demo` 坑） |
| 串口读到一坨错误/卡住 | ISR 挂起的接收错误未处理 | `Uart::take_error()` 按溢出>帧错>校验>噪声优先级取走（出处：cw32l012-hal CHANGELOG 0.9.0） |
