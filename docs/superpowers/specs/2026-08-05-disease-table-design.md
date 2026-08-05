# 病菌模块·阶段 1：病菌表解析 + 转移补强（设计规格，2026-08-05）

## 目标与范围

**本阶段**（对照原版 simDLL）：
1. `CreateDiseaseTable` 解析（原版 `Disease::Disease` 构造，11_msvcrt_ignored.c
   L25718-26091；消息 id 0x31311BAF = 825301935，handler 已注册为空桩）；
2. `gDisease` 全局挂接（`globals::G_DISEASE` 已存在，`Mutex<SendSyncPtr<Disease>>`）；
3. `AddDiseaseToCell` 补强——原版**病菌强度混合**（L26163-26263），替换当前
   "同累加/异替换"简化版。

**不做**（后续阶段）：格子病菌模拟 UpdateCells/PostProcess/CalculateFinalDiseaseCount
（阶段 2）、DiseaseEmitter/Consumer/ConsumeDisease/CellDiseaseModification 消息组件
（阶段 3）、辐射病菌贡献（阶段 4，依赖病菌表，本阶段后解锁）。

## 消息数据格式（C# SimMessages.CreateDiseaseTable → sim）

```
count          i32   （病菌数）
elements_count i32   （元素表数，校验用）
repeat count 次：
  name             KleiString（u32 字节长度 + UTF-8）
  hash_id          u32
  strength         f32
  temperature_range      RangeInfo（4×f32：minViable/minGrowth/maxGrowth/maxViable）
  temperature_half_lives RangeInfo
  pressure_range         RangeInfo
  pressure_half_lives    RangeInfo
  radiation_kill_rate    f32
  repeat elements_count 次（ElemGrowthInfo，Klei.AI.DiseaseGrowthRules）：
    under_population_death_rate f32
    population_half_life        f32
    over_population_half_life   f32
    diffusion_scale             f32
    min_count_per_kg            f32
    max_count_per_kg            f32
    min_diffusion_count         i32
    min_diffusion_infestation_tick_count u8
```

（C# 写入顺序与反编译 `Disease::Disease` 读取顺序逐字段一致：0x30 ReadUInt(hash) →
0x18 ReadFloat × 17（strength + 4×RangeInfo）+ 0x18(radiationKillRate@0x148) →
ElemGrowthInfo 8 表 resize 到 elements_count 逐项读：6×ReadFloat + ReadInt(0x38) +
ReadByte(0x50)。）

## 实现要点

### 1. Disease 表解析（b_elements/disease.rs 扩展）

- 现有 `DiseaseInfo`（336B）/ `ElemGrowthInfo`（256B）/ `RangeInfo`（16B）布局已
  精确，无需改动结构。
- 新增 `Disease::from_stream(reader) -> Result<Disease, BufferError>`（或模块级
  解析函数）：
  - 读 count / elements_count；
  - 校验 `elements_count == 元素表数量`：原版 DebugBreak（仅调试器附加）+
    KCrashReporterReportMessage；本项目 warn 不中断（与 CreateElementsTable 同款
    处理）；
  - `diseases.resize(count)` / `disease_names.resize(count)`（MsvcVector 容量感知）；
  - 逐病菌按上述格式读：名称 → hash → strength → 4×RangeInfo →
    radiation_kill_rate → 8 个 ElemGrowthInfo vector resize 到 elements_count 逐项读。
- `GetDiseaseIndex` 已实现（线性搜索，L26386）。

### 2. gDisease 全局挂接（save_load.rs）

- `handle_create_disease_table` 空桩 → 调解析，构造 `Box<Disease>` 替换
  `globals::G_DISEASE`（释放旧 Box，对齐原版 CreateDiseaseTable 的
  `unique_ptr` 替换 + 析构）。
- `clean_up` 已释放 G_DISEASE（save_load.rs L54）✓。
- `disease_ptr` 读取路径（save_load BeginSave L1660 等）保持。

### 3. AddDiseaseToCell 强度混合（liquid_flow.rs）

原版 L26163-26263 逐分支：
```
cur = cells.disease_idx[cell]; cur_count = disease_count[cell]
if cur == new_idx:
    result = cur_count + new_count; result_idx = cur       // 同病菌累加
else:
    result = new_count; result_idx = new_idx               // 默认替换
    if cur != 0xff && new_idx != 0xff:                     // 双方真实病菌 → 强度混合
        result = cur_count; result_idx = cur               // 默认保留当前
        f10 = cur_count × strength[cur]; f11 = new_count × strength[new]
        if f10 <= f11:
            if cur_count < 0: result = -cur_count; result_idx = new_idx
            // 否则保留当前（f10<=f11 且 cur>=0）
        else:
            d = (int)(new_count − (f10/f11) × cur_count)
            if d < 0: new_idx = cur_idx                     // 混合差值负 → 当前胜
            result = -d; result_idx = new_idx
            if -1 < d: result = d
写回 idx/count
if result > 0:
    if result_idx != cur: infestation_tick_count[cell] = 0
    return
清除四字段（idx=0xff, count=0, infestation=0, growth_error=0）
```

注意 f10/f11 用**当前病菌与新病菌各自 strength**（依赖 gDisease 表，本阶段解析后
可用）；`diseases` 索引越界（理论不发生的脏数据）防御为保留当前（原版越界断言）。

## 测试计划

1. **消息解析**：按 C# 写入格式构造字节（1 个病菌 + 2 元素）→ 断言 Disease 内容
   （hash/strength/ranges/radiationKillRate/elemGrowth 全字段）。
2. **校验**：elements_count 与元素表不匹配 → warn、不 panic、表仍可读。
3. **AddDiseaseToCell**：同累加 / 异替换（一方 0xff）/ 强度混合（f10<=f11、
   f10>f11 两向 + 负 count）/ count≤0 清四字段。
4. **CleanUp 释放**：替换 G_DISEASE 后 clean_up 不崩。

## 回归与安全

- 现有病菌简化转移（同累加/异替换）路径升级为强度混合后，液体/气体/碎片转移
  行为变化——全量测试回归确认无断言破坏。
- 全量测试串行 + release 构建。
