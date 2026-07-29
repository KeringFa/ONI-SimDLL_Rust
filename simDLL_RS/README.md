# simDLL_RS

Rust 重写《缺氧》(Oxygen Not Included) 游戏原版 `simDLL.dll`。

## 当前状态：Phase 0 基础骨架

- 9 个 `SIM_*/SYSINFO_*` 桩函数已导出
- Logger 双输出到 `./logs/simdll.log` 和 stderr
- CrashDumpHandler 写 minidump 到 `./logs/SimDLL_CRASH_*.dmp`
- 游戏加载此 DLL 可进入主菜单，加载存档会失败（预期）

## 构建

```
cargo build --release --target x86_64-pc-windows-msvc
```

产出：`target/x86_64-pc-windows-msvc/release/SimDLL.dll`

## 测试

```
cargo test
```

## 部署

用户自行将 `SimDLL.dll` 复制到游戏 `OxygenNotIncluded_Data/Plugins/x86_64/` 覆盖原版。

## 设计文档

- 总体规格：`../docs/superpowers/specs/2026-07-29-simdll-rs-foundation-design.md`
- 实现计划：`../docs/superpowers/plans/2026-07-29-simdll-rs-phase0-foundation.md`
- 路线图：`docs/roadmap.md`
- 源码分段索引：`../OxygenNotIncluded_Source/simDLL_edit/INDEX.md`
