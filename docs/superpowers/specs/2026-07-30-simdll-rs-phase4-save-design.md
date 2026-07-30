# Phase 4 — 存档保存（SIM_BeginSave / SIM_EndSave）设计规格

**日期**：2026-07-30
**阶段**：Phase 4（存档保存）
**前置**：Phase 3 完成（commit `2bcb626`，183 测试通过）

## 1. 目标

实现 `SIM_BeginSave` 与 `SIM_EndSave`，与原版 simDLL save 文件**二进制兼容**。

**验收标准**（roadmap.md L83-85）：
- ✅ Rust simDLL 保存的存档能被原版 simDLL 加载
- ✅ 存档字节序、字段顺序与原版完全一致
- ✅ Rust↔Rust round-trip 测试可行（需修 Phase 3 Load bug，见 §4）

## 2. 源码对照

### 2.1 SIM_BeginSave（01_sim_api.c L7-171）

```c
schar * __cdecl SIM_BeginSave(int *param_1, int param_2, int param_3)
{
  // 1. 若 SaveBuffer 已存在，先析构并置 null
  if (SaveBuffer != NULL) { (*SaveBuffer->vtable)(SaveBuffer, 1, ...); SaveBuffer = NULL; }

  // 2. 计算所需 buffer 大小
  iVar11 = gSimData->height * gSimData->width;          // total_cells
  uVar15 = iVar11 * 0x24 + 0x1d;                        // cells*36 + 29 (header)
  *param_1 = uVar15;                                    // 写入 out_size

  // 3. 分配 Buffer（operator_new(0x20) + Buffer::Buffer(size)）
  pBVar7 = operator_new(0x20);
  pBVar8 = (pBVar7 != NULL) ? Buffer::Buffer(pBVar7, uVar15) : NULL;
  SaveBuffer = pBVar8;

  // 4. 创建 BinaryBufferWriter 包装 SaveBuffer
  BinaryBufferWriter::BinaryBufferWriter(&local_40, pBVar8);

  // 5. 写头部（L46-52）：8 + 4*5 + 1 = 29 字节
  WriteBytes(&local_40, 8, "SIMSAVE");                 // 魔数 "SIMSAVE\0"
  WriteInt(&local_40, 0xf);                            // version = 15
  WriteInt(&local_40, gSimData->width);                // 内部宽
  WriteInt(&local_40, gSimData->height);               // 内部高
  WriteInt(&local_40, param_2);                        // p2
  WriteInt(&local_40, param_3);                        // p3
  WriteByte(&local_40, gSimData->saved_options);       // 1 byte @ SimData+0x18

  // 6. 三个循环，每个 cell 共写 36 字节
  // === Loop 1（L55-89）：每 cell 16 字节（4*4）===
  for (i = 0; i < total_cells; i++) {
    uVar14 = updated_cells->element_idx[i];            // u16
    WriteInt(gElements[uVar14].id);                    // 4B 元素 hashcode
    WriteFloat(updated_cells->temperature[i]);         // 4B
    WriteFloat(updated_cells->mass[i]);                // 4B
    WriteFloat(updated_cells->radiation[i]);           // 4B
  }

  // === Loop 2（L90-117）：每 cell 8 字节（4+4）===
  for (i = 0; i < total_cells; i++) {
    bVar1 = updated_cells->disease_idx[i];             // u8
    if (bVar1 != 0xff) {
      // 查 disease 表：name_hash = gDisease->diseases[bVar1].name_hash
      pBVar8 = gDisease->diseases[bVar1].name_hash;    // u32
    } else {
      pBVar8 = 0;                                      // 无 disease 写 0
    }
    WriteUInt(pBVar8);                                 // 4B disease hashcode
    WriteInt(updated_cells->disease_count[i]);         // 4B
  }

  // === Loop 3（L118-157）：每 cell 12 字节（4+4+4）===
  for (i = 0; i < total_cells; i++) {
    uVar4 = backwall->element_idx[i];                  // u16
    if (uVar4 == 0xffff) {
      uVar4 = GetElementIndex(0x2d39bf75);            // vacuum hashcode
      // 副作用：清零 backwall mass[i] 与 temperature[i]
      backwall->mass[i] = 0;
      backwall->temperature[i] = 0;
    }
    WriteInt(gElements[uVar4].id);                      // 4B
    WriteFloat(backwall->mass[i]);                     // 4B
    WriteFloat(backwall->temperature[i]);              // 4B
  }

  // 7. 校验 SaveBuffer->IsValid + size == uVar15
  // 8. 返回 SaveBuffer->data_ptr（C# 拷贝后调 SIM_EndSave）
  BinaryBufferWriter::~BinaryBufferWriter(&local_40);
  return SaveBuffer->data_ptr;
}
```

### 2.2 SIM_EndSave（01_sim_api.c L195-204）

```c
void __cdecl SIM_EndSave(void)
{
  if (SaveBuffer != NULL) {
    (*SaveBuffer->vtable)(SaveBuffer, 1);              // 析构
    SaveBuffer = NULL;
  }
}
```

### 2.3 每字节布局

**头部（29 字节）**：
| offset | 字段 | 大小 | 说明 |
|--------|------|------|------|
| 0-7 | magic | 8 | `"SIMSAVE\0"` = `53 49 4d 53 41 56 45 00` |
| 8-11 | version | 4 | `0x0f` = 15（little-endian u32） |
| 12-15 | width | 4 | gSimData->width（i32 LE） |
| 16-19 | height | 4 | gSimData->height（i32 LE） |
| 20-23 | p2 | 4 | SIM_BeginSave 参数 2 |
| 24-27 | p3 | 4 | SIM_BeginSave 参数 3 |
| 28 | saved_options | 1 | gSimData->saved_options（@ SimData+0x18） |

**Cell 数据（每 cell 36 字节，循环 total_cells 次）**：

Loop 1（offset 29 + i*36 起 16 字节）：
| offset | 字段 | 大小 | 来源 |
|--------|------|------|------|
| 0-3 | element_id | 4 | gElements[updated_cells.element_idx[i]].id |
| 4-7 | temperature | 4 | updated_cells.temperature[i]（f32 LE） |
| 8-11 | mass | 4 | updated_cells.mass[i]（f32 LE） |
| 12-15 | radiation | 4 | updated_cells.radiation[i]（f32 LE） |

Loop 2（offset 29 + i*36 + 16 起 8 字节）：
| offset | 字段 | 大小 | 来源 |
|--------|------|------|------|
| 0-3 | disease_hash | 4 | gDisease.diseases[disease_idx[i]].name_hash，或 0 若 disease_idx==0xff |
| 4-7 | disease_count | 4 | updated_cells.disease_count[i]（i32 LE） |

Loop 3（offset 29 + i*36 + 24 起 12 字节）：
| offset | 字段 | 大小 | 来源 |
|--------|------|------|------|
| 0-3 | backwall_element_id | 4 | gElements[backwall.element_idx[i]].id；若 idx==0xffff 替换为 vacuum idx |
| 4-7 | backwall_mass | 4 | backwall.mass[i]（f32 LE） |
| 8-11 | backwall_temperature | 4 | backwall.temperature[i]（f32 LE） |

**总大小验证**：`29 + width*height*36 = 0x1d + total_cells*0x24` ✓ 与源码 `uVar15 = iVar11 * 0x24 + 0x1d` 一致。

## 3. 架构设计

### 3.1 模块组织（决策 D1）

新增逻辑放在 [save_load.rs](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/save_load.rs)，与 Phase 3 Load 模式对称：
- `save_load::begin_save(p2: i32, p3: i32) -> Result<*mut u8, SaveError>` — 构造 save buffer，存入全局，返回数据指针
- `save_load::end_save()` — 释放全局 SaveBuffer

[sim_api.rs](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/sim_api.rs) 的 `SIM_BeginSave` / `SIM_EndSave` 从 Phase 0 桩升级为真实实现，仅做 FFI 包装（catch_unwind + 调用 save_load）。

**理由**：与 Phase 3 Load（逻辑在 save_load.rs，sim_api.rs 仅 FFI 包装）保持一致。便于后续 Phase 6 修改 dispatch_save_load_message 时一处集中维护。

### 3.2 SaveBuffer 所有权（决策 D2）

新增全局：
```rust
pub static G_SAVE_BUFFER: Lazy<Mutex<Option<Box<[u8]>>>> =
    Lazy::new(|| Mutex::new(None));
```

- `begin_save`：先 take 旧的（drop），构造新的 `Vec<u8>` → `into_boxed_slice()` → 存入全局 → 返回 `box.as_ptr()`
- `end_save`：`*g = None`（drop Box）
- 指针有效期：从 `begin_save` 返回到 `end_save` 调用之间（单线程契约，与原版一致）

**不使用 leak**：避免内存泄漏（每次保存都 leak 8MB 不可接受）。

### 3.3 错误处理（决策 D3）

源码用 `_invalid_parameter_noinfo_noreturn()` 终止进程。Rust 版：
- 元素索引/disease 索引越界 → 用 `MsvcVector::get()`（越界返回 `T::default()`）+ `ElementsTable::elements.get(idx)`（越界返回 None，写入 0）
- 不 panic，记录 warn 日志
- **运行时差异**：原版崩溃 → Rust 写入默认值（0）。仅影响损坏数据，正常存档不会触发。

## 4. Phase 3 Load 修复（决策 D4，用户已批准）

### 4.1 当前 bug

[save_load.rs:384-396](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/save_load.rs#L384-L396) 多读 1 个 int：

```rust
// 当前（错误）：
inner_w = reader.read_int()?;       // L36
inner_h = reader.read_int()?;       // L37（实际只有 1 次 L37 调用）
start_offset = reader.read_int()?;  // 第 3 次 read，但源码只有 2 次！
local_60 = reader.read_int()?;      // L40
let _extra = reader.read_int()?;    // L41
save_settings_byte = reader.read_byte()?;
```

### 4.2 修复后

```rust
// 修复后（version<0x10 块内 4 ints + 1 byte；含 version 共 5 ints + 1 byte，与 SIM_BeginSave 写入对称）：
inner_h = reader.read_int()?;       // L36 → local_res18[0] → iVar8（inner loop bound）
start_offset = reader.read_int()?;  // L37 → local_res20 → iVar9（outer loop bound）
local_60 = reader.read_int()?;      // L40 → local_60（if version > 0xd）
let _extra = reader.read_int()?;    // L41 → discarded（if version > 0xd）
save_settings_byte = reader.read_byte()?;  // L45（if version > 0xc）
```

**变量映射依据**（来自源码 L51-52 的使用点，非 Ghidra 推测）：
- 源码 `iVar8 = local_res18[0]`（L52）→ 用作内层循环 bound（L57 `if (0 < iVar8)`）
- 源码 `iVar9 = local_res20`（L51）→ 用作外层循环 bound（L53 `if (local_res20 != 0)`、L49 外层 `while`）

Phase 3 Rust 变量按使用语义命名：
- `inner_h` → 内层循环 bound（[save_load.rs:419](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/save_load.rs#L419) `if inner_h > 0`、L421、L438）
- `start_offset` → 外层循环 bound（[save_load.rs:413](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/save_load.rs#L413)、L417、L450、L462、L471）

故 L36 read 应赋给 `inner_h`（非 `inner_w`），L37 read 应赋给 `start_offset`。`inner_w` 在 Phase 3 全程未被循环使用（仅 L404 日志），修复时移除。

### 4.3 字段语义重新映射

源码 L36/L37 两次读取的目标变量，依据源码 L51-52 的使用点确定（**非** Ghidra 推测，无二义性）：
- L36 read → `local_res18[0]` → `iVar8`（内层循环 bound，语义 = "height"）
- L37 read → `local_res20` → `iVar9`（外层循环 bound，语义 = "start_offset"）

修复时选择映射：`inner_h = L36 read`，`start_offset = L37 read`。保留 Phase 3 的 `start_offset`/`inner_h` 变量命名以减少测试改动，移除未使用的 `inner_w`。

**实施细节**：
1. 移除 [save_load.rs:385](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/save_load.rs#L385) `inner_w = reader.read_int()?;`（该变量未被循环使用）
2. 将 L386 `inner_h = reader.read_int()?;` 的注释改为 `// L36 → local_res18[0] → iVar8（inner bound）`
3. L387 `start_offset = reader.read_int()?;` 注释改为 `// L37 → local_res20 → iVar9（outer bound）`
4. 更新 [save_load.rs:404](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/save_load.rs#L404) 日志，移除 `inner_w` 字段
5. 循环逻辑（L413-446）使用 `inner_h` 和 `start_offset`，**变量用法不变**，无需改动

### 4.4 测试影响

- lib 测试 `save_load.rs::tests::load_*` 可能需更新（具体看哪些测试断言 inner_w/inner_h/start_offset 的具体值）
- 集成测试 `save_load_smoke.rs` 不应受影响（仅做 does_not_panic / 魔数校验）

## 5. 测试策略

### 5.1 单元测试（save_load.rs::tests）

- `begin_save_empty_world_returns_header_only`：SimData width=0/height=0 → 返回 29 字节，验证魔数 + version + 字段
- `begin_save_one_cell_writes_36_bytes`：mock 1 个 cell，验证 36 字节内容（element_id/temp/mass/radiation/disease_hash/disease_count/backwall_id/backwall_mass/backwall_temp）
- `begin_save_disease_idx_0xff_writes_zero_hash`：disease_idx=0xff → disease_hash=0
- `begin_save_backwall_0xffff_replaces_with_vacuum`：backwall idx=0xffff → 替换为 vacuum idx + 副作用清零 mass/temp
- `end_save_releases_buffer`：begin_save 后 end_save → G_SAVE_BUFFER 变 None
- `begin_save_twice_drops_first_buffer`：连续两次 begin_save → 第二次成功

### 5.2 集成测试（save_load_smoke.rs 新增或独立文件）

- `sim_begin_save_returns_non_null_and_correct_size`：empty SimData → *out_size == 29，返回非空指针
- `sim_begin_save_with_null_out_size_does_not_panic`：null out_size 仍返回非空指针
- `sim_end_save_after_begin_save_no_op`：begin_save + end_save + end_save → 不 panic
- `sim_begin_save_after_shutdown_safe`：SIM_Shutdown 后调 SIM_BeginSave → 不 panic（catch_unwind 兜底）

### 5.3 Round-trip 测试（依赖 D4 修复）

- `save_then_load_round_trip`：mock SimData（2×2 cells + 1 backwall） → begin_save → load 回来 → 验证 SimData 字段一致

### 5.4 既有测试更新（sim_api.rs 单元测试）

> 注：§5.2 是 `tests/save_load_smoke.rs` 集成测试（新文件或扩展现有）；本节是 `src/sim_api.rs::tests` 模块内的单元测试更新（[sim_api.rs:375-386](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/sim_api.rs#L375-L386)）。两者模块不同，可重名。

- `sim_begin_save_returns_null_and_zero_size` → 改名 `sim_begin_save_returns_non_null_and_correct_size`，断言从 `is_null() && size==0` 改为 `!is_null() && size==29`（empty SimData）
- `sim_begin_save_with_null_out_size_does_not_panic` → 保留，断言从 `is_null()` 改为 `!is_null()`（null out_size 仍返回有效指针）
- `sim_end_save_does_not_panic` → 保留（[sim_api.rs:389-391](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/sim_api.rs#L389-L391)）

## 6. 实现步骤（高 level，详细 plan 由 writing-plans 生成）

1. 修 Phase 3 Load 多读 bug（移除 `inner_w = reader.read_int()?;` 一行，将 `inner_h` 改读 L36，`start_offset` 保持读 L37，详见 §4.3）
2. 跑 cargo test 确认 Phase 3 测试仍通过（更新日志断言以匹配新读法）
3. 在 save_load.rs 加 `G_SAVE_BUFFER` 全局 + `begin_save` + `end_save` 函数
4. 在 sim_api.rs 把 `SIM_BeginSave` / `SIM_EndSave` 桩替换为真实实现
5. 加单元测试（§5.1）
6. 加集成测试（§5.2）
7. 加 round-trip 测试（§5.3）
8. 跑 `cargo test` + `cargo build --release --target x86_64-pc-windows-msvc`
9. commit（用 `git commit -F` 临时文件方式，PowerShell 执行策略禁用）
10. 通知用户重新部署测试

## 7. 已知问题与遗留

### 7.1 BackwallSOA struct 布局差异（不修，仅记录）

Rust BackwallSOA（[sim_data.rs:212-220](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/sim_data.rs#L212-L220)）有 4B 隐式 padding（u16+i16 后对齐 MsvcVector 到 8B），导致 element_idx MsvcVector 起始偏移为 0x8（而非源码的 0x4）。

**对 Phase 4 影响**：无。Phase 4 通过 Rust 字段名访问（`backwall.element_idx.get(i)`），不通过裸偏移。Save 写入的字节值正确，与源码一致。

### 7.2 DiseaseInfo struct stride 差异（不修，仅记录）

源码 DiseaseInfo stride = 0x150 = 336 字节，Rust DiseaseInfo 仅 20 字节（[disease.rs:18-24](file:///c:/Users/KeringFa/Desktop/simDLL_RS_Kit/simDLL_RS/src/disease.rs#L18-L24)）。

**对 Phase 4 影响**：无。Phase 4 通过 `disease.diseases[idx].name_hash` 字段访问，不通过裸偏移。Phase 3 stub 让 disease_idx 全为 0xff，所以 Phase 4 save 时实际不会触发 disease 表查找（disease_hash 永远写 0）。Phase 6+ 真实 disease 表接入时复核 stride。

### 7.3 Phase 6+ 仍 stub 的依赖

- `Sim::Start` 是 stub（无模拟线程）→ 游戏仍会卡在加载后无法 tick
- `SimFrameManager::HandleMessage` 未实现 → cell 数据无法通过 ModifyCell 消息接收
- 这两项是 Phase 6 范畴，Phase 4 不处理（用户已确认）

## 8. 运行时体验差异

### 8.1 用户可见变化

| 场景 | Phase 3（当前） | Phase 4（目标） |
|------|-----------------|-----------------|
| 点击"保存"按钮 | SIM_BeginSave 返回 null，C# 收到 null → 游戏可能弹"无法保存"或静默失败 | SIM_BeginSave 返回有效指针 + size → C# 拷贝字节写入存档文件 → 保存成功 |
| 存档可被原版加载 | N/A | ✅ 是（字节序与原版一致） |
| 存档可被 Rust 加载 | N/A | ✅ 是（修 Phase 3 Load bug 后） |

### 8.2 不变项

- 加载流程不变（仅修 1 个 int 多读 bug，逻辑等价）
- 模拟线程未启动（Phase 6 范畴）
- cell 数据仍来自消息分派（Phase 6 范畴）

## 9. 决策摘要

| ID | 决策 | 选择 | 理由 |
|----|------|------|------|
| D1 | 模块组织 | save_load.rs 加 begin_save/end_save | 与 Phase 3 Load 对称，sim_api.rs 仅 FFI 包装 |
| D2 | SaveBuffer 所有权 | 全局 Mutex<Option<Box<[u8]>>> | 避免 leak，单线程契约与原版一致 |
| D3 | 错误处理 | 越界返回默认值 + warn 日志 | 不崩溃，原版崩溃 → Rust 默认值（仅影响损坏数据） |
| D4 | Phase 3 Load bug | 一并修复 | 用户已批准；保证 Rust↔Rust round-trip 可行；与源码严格对齐 |
| D5 | 循环结构 | 3 个独立循环 | 严格镜像源码，便于逐行验证 |
| D6 | 不修 BackwallSOA/DiseaseInfo 布局 | 仅记录 | Phase 4 不通过裸偏移访问，无影响 |
