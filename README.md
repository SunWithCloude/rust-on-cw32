# Rust on CW32

以 **CW32L012**（武汉芯源 CMSemicon，Cortex-M0+）为教具的嵌入式 Rust 中文教程——
从 PAC 裸寄存器点到阻塞式 HAL，再到 embassy 异步外设全家桶。
**书中每一段代码都是本仓 `examples/` 下的真实例程**，CI 在
`thumbv6m-none-eabi` 上持续编译。

在线阅读：https://sunwithcloude.github.io/rust-on-cw32/

## 生态地图

| 仓库 | 角色 |
|------|------|
| [Cw32l012_pac](https://github.com/SunWithCloude/Cw32l012_pac) | L012 寄存器层（手写转录自用户手册 V1.4） |
| [Rust_Cw32L012](https://github.com/SunWithCloude/Rust_Cw32L012) | `cw32l012-hal` 阻塞式 HAL（自带运行时，crates.io 已发布） |
| [Cw32l012_embassy](https://github.com/SunWithCloude/Cw32l012_embassy) | `embassy-cw32` 异步 HAL（11 家族，embassy 执行器） |
| [cw32-pacs](https://github.com/SunWithCloude/cw32-pacs) | 其余 10 家族的 PAC |
| [cw32-bsp](https://github.com/SunWithCloude/cw32-bsp) | 11 家族 BSP（引脚映射 + 工程模板） |

## 本地构建

```bash
# 生态依赖按路径挂在本仓上一级
git clone https://github.com/SunWithCloude/Cw32l012_pac.git      ../cw32l012-pac
git clone https://github.com/SunWithCloude/Rust_Cw32L012.git     ../cw32l012-hal
git clone https://github.com/SunWithCloude/Cw32l012_embassy.git  ../embassy-cw32

cargo install mdbook
mdbook build                                     # 书 → book/
cargo build --workspace --target thumbv6m-none-eabi --release --examples
```

写作规范见 [CONVENTIONS.md](CONVENTIONS.md)。
