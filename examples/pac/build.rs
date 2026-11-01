// 把 memory.x 暴露给链接器；cortex-m-rt 的 link.x 会 INCLUDE memory.x。
// 注意：链接脚本参数在 build.rs 里用 rustc-link-arg-bins 下发——
// .cargo/config.toml 的 rustflags 只在"以本 crate 目录为 CWD 构建"时
// 生效，从工作区根/CI 构建时 cargo 根本不会读子目录里的 config。
use std::env;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=memory.x");
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rustc-link-search=native={}", manifest.display());
    println!("cargo::rustc-link-arg=-Tlink.x");
}
