# 光照模块对照审查 + 验证准备（2026-08-04）

**范围**：sim 侧光照全量核对——`update_exposed_to_sun_property_texture`
（对照 SimDLL_Source.c L145676-145960）、ElementLightAbsorptionData 映射、
`sim.worlds` 填充路径（DefineWorldOffsets）、GameData 曝光缓冲、调用点接线、
C# 消费链路（Grid.LightIntensity / LogicLightSensor / BristleBlossom）。

## 结论：代码与原版逐行一致；运行时发现并修复 1 个 NaN 根因 bug（用户实测验收）；
无功能缺口；注释方向标签勘误已修。

## 核对通过

1. **曝光纹理算法**（L145676-145960）逐行一致：
   - 按 `sim.worlds`（16B/条：offset_x/offset_y/size_x/size_y）逐世界遍历；
   - 世界**顶部行**（= 天空行，game 行 w_y+w_h-1）先填 0xff（满曝）；
   - 每列阳光缓冲初始 1.0；行循环 `w_h-2 → 0`（top→bottom）；
   - 不透明分支（properties & 0x30 == 0x30）：不透明元素 hash `0x2531469c` 的
     massScale(+4) × 上方格质量 → clamp 1.0 → `sun -= absorb × factor(+0)`；
   - 普通分支：上方格（cell+width）元素/质量同样衰减；`properties & 0x20` 置位
     时该列归零（f23 写 sun 缓冲、f22 写纹理——两值不同，与原版一致）；
   - 写回 `exposed[game] = (f22×255) as u8`，sim→game 去边界换算一致。
2. **ElementLightAbsorptionData**（L13300-13309）：factor@0 / massScale@4（8B），
   Rust 结构体映射一致。
3. **sim.worlds 单星填充**：C# `SimMessages.DefineWorldOffsets` 在 SaveLoader
   L443（读档）与 L1032（新档）都会发送（`ClusterManager.WorldContainers` 迭代，
   单星=1 条）；消息 id -895846551 == 3399120745（u32）已注册到 handler 表；
   handler 解析 count + 每世界 4×i32 与原版 02_save_load.c L297-342 一致。
4. **GameData 缓冲**：property_texture_exposed_to_sunlight 已分配（game 格数 u8）。
5. **调用点接线**：copy_sim_data_to_game 内按原版顺序 Flow → Exposed → Liquid
   （原版 L132388-132390）；GameSync 双缓冲交换后 C# 读取 m_game_data 指针
   （Game.cs L1076/L1082）。
6. **C# 消费链**：`Grid.LightIntensity[i] = exposed[i]/255 × world.currentSunlightIntensity
   + LightCount[i]`（Grid.cs L558-564）；LogicLightSensor 每 sim 更新采样
   （LogicLightSensor.cs L109）；毛刺花等植物读同一值。生物光/建筑光/装饰光
   全部走 C# LightCount（Light2D），与 simDLL 无关。

## 勘误（已修，纯注释）

- "下方格（cell+width）" → **上方格**：原版曝光函数读 cell+width 作为吸光格
  （阳光自上而下先经过），行循环 top→bottom；本项目液体约定 below=cell-width，
  二者一致（更大索引 = 更高行）。此前 property_texture.rs 头部、函数体、测试
  注释均误标"下方格"，已全部更正（变量 below → above_cell）。液体纹理里同类
  标注亦为误标（原版液体纹理读 cell+width 做表面检测），不在本模块范围，
  已记录待液体纹理后续顺带更正。

## 运行时根因修复（2026-08-04，用户实测：太阳光只亮顶部行 → 修复后正常）

**根因（NaN 毒化阳光列）**：
- 真空 `defaultMass=0` → `massScale = 1.0/0.0 = INFINITY`（03_elements.c
  L362-368 反编译：非固体 = 1/defaultMass）；
- 空间生态区真空格质量=0 → `absorb = massScale × mass = INFINITY × 0 = NaN`；
  `absorb >= 1.0` 对 NaN 恒 false → NaN 进入 `sun -= absorb×factor` 与列缓冲
  → 整列阳光被毒化 → 除顶部填充行（0xff）外全 0；
- 流星雨期间空间区流星尾气质量>0 → 该列 absorb 有限 → 阳光可传播（用户观察
  "尾气能被照亮"恰好验证真空是元凶）。

**证据链**：
- 存档解析（03_only.sav）：世界顶行 sim 384 = 真空质量 0；世界 (0,0,256,384)
  与 DefineWorldOffsets 一致；worlds 时序（首帧空）原版同款（C# Sim.Start
  先于 DefineWorldOffsets），非根因；
- old 稳定版对 `mass_per_unit==0` 用 multiplier=1.0（默认值）→ 无 NaN →
  光照正常——本修复同款；
- 此前曝光单测把真空 defaultMass 设 1.0 掩盖了 bug；新回归用真实 0 值。

**修复**：
1. `elements_table.rs`：非固体 `defaultMass > 0` → 1/mass；`<= 0`（真空等）
   → massScale=1.0（固体仍 INFINITY 全遮光）；
2. `property_texture.rs`：曝光吸收 `absorb.is_nan() → 0.0`（防御，双分支）；
3. 回归测试 `exposed_sun_passes_through_vacuum_space_biome`（全真空列=255）。

## 完整性确认（sim 侧光照就这些）

- 原版 05_liquid_flow.c 中光照相关仅 `UpdateExposedToSunPropertyTexture`
  （L7-226）+ gElementLightAbsorptionData（L133-186 引用）+ 液体纹理读曝光
  缓冲（L356）；无其他 sim 侧光照函数/消息。
- 生物光/建筑光/装饰光/太阳光渲染全部在 C# Light2D（LightCount），simDLL
  不参与（Grid.cs L558-564）。
- ActiveRegion.currentSunlightIntensity 由 NewGameFrame 解析（原版只存储，
  UpdateData 不使用——sunshine.txt 核对一致）。

## 验证

- 全量 451 测试串行全过（399 lib + 52 集成）；release 构建成功。
- 曝光纹理单测 8 条（含衰减列 127/继承 127/无遮挡 255/真空空间区 255）。
- **用户实测验收（2026-08-04）**：太阳光恢复正常；空间生态区满曝光、
  阳光穿透真空；流星尾气可被照亮（验证真空 NaN 修复）；光线传感器/毛刺花
  正常。

## 遗留（Minor，低置信度，不影响当前行为）

- `0x2531469c`（props&0x30 分支的不透明元素 hash）不在 SimHashes 枚举中；
  我们 `get_element_index_pub` 返回 INVALID → 分支用 (0.0,0.0)（不遮光），
  old 版同用真空（不遮光），行为一致。原版若该 hash 有效则用其吸光数据——
  仅在 PartialLightBlocking（0x30）建筑格触发，暂不影响实测，记录在案。
