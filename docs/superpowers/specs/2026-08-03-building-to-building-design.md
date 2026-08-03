# BuildingToBuildingHeatExchange（导热板 ContactConductivePipeBridge）严格对照实现设计

**日期**：2026-08-03
**状态**：待用户确认
**类别**：C1 消息/组件/句柄层 · 建筑↔建筑换热（导热板）

## 1. 背景

温度模块已实现：建筑温度链路、管道换热、方格↔方格+背墙、建筑↔格子、ElementChunk，并经全量对照审查。
下一步：**导热板（ContactConductivePipeBridge）**——建筑↔建筑换热组件 `BuildingToBuildingHeatExchange`
（原版 L138358-138640），中心格可与任何事物换热、出入口只与方块换热（接触列表由 C# 注册，sim 只管配对换热）。

变温板（ThermalBlock）已确认纯 C#（OverrideExtents 扩 3×3，sim 侧标准 AddBuildingHeatExchange），**不做**。

本设计严格对照原版 `SimDLL_Source.c`：
- `BuildingToBuildingHeatExchange::Register/Add/Remove/Unregister/Update`（L138121/137831/138193/138337/138358）；
- ProcessFrame 消息段（L116540-116650）；
- 依赖 `BuildingHeatExchange::ChangeBuildingTemperature`（L136853）、`GetSimHandleForBuildingHandle`（L137051）。

## 2. 架构认知

### 2.1 数据（BuildingToBuildingHeatExchangeData，40B = 0x28）

| 偏移 | 字段 | 说明 |
|------|------|------|
| 0x00 | heatExchange_handle | 关联的建筑温度句柄（Update 用它查 buildingHeatExchange 数据） |
| 0x10 | inContactBuildings | vector<{buildingInContact handle, cellsInContact}>，8B/条 |

注册表模式同 BuildingTemperatureManager：版本句柄、free list、swap-remove。

### 2.2 消息（对照 C# SimMessages 与 ProcessFrame 核实）

| 消息 | ID | 尺寸 | 内容 |
|------|-----|------|------|
| Register（AddBuildingToBuildingHeatExchange） | 2956249079 | 8B | {callbackIdx, heatExchange_handle} |
| RemoveInContact（modifies） | 2301110083 | 8B | {self_handle, buildingInContact} |
| AddInContact（AddBuildingToBuildingHeatExchange 附加） | 2708242975 | 12B | {self_handle, buildingInContact, cellsInContact} |
| Remove（RemoveBuildingToBuildingHeatExchange） | 697100730 | 8B | {callbackIdx, handle} |

ProcessFrame 顺序（L116540-116650）：adds(Register) → modifies(RemoveInContact) → addBuildingInContact(Add) → removes(Unregister)。

**现状问题**：frame_manager 把 Register/RemoveInContact/Remove 丢弃在 C2 桩分支，且 MSG_ADD_BUILDING_IN_CONTACT 按 **8B** emplaces（应为 12B）——需修正。

### 2.3 语义

- Register：校验 heatExchange_handle 有效 → 分配记录（空接触列表）→ callbackIdx≠-1 时推 componentStateChanged{callbackIdx, 新句柄}。
- Add：按 self_handle 找记录，追加 {buildingInContact, cellsInContact} 到接触列表（SetData 写回）。
- RemoveInContact：按 self_handle 找记录，从接触列表移除 buildingInContact。
- Unregister：释放记录（版本+1、free list、swap-remove）。

## 3. 实现项

### 3.1 `BuildingToBuildingHeatExchangeManager`（c_simulation/building_to_building.rs）

注册表 + 消息方法（Register/Add/RemoveContact/Unregister），模式同 element_chunk.rs。
`heatExchange_handle` 写入点实现期按 L138121/L138470 核对（Update 将 record@0 作为建筑句柄查 buildingHeatExchange）。

### 3.2 依赖：`change_building_temperature` / `get_sim_handle_for_building_handle`

- `building_temperature.rs` 新增 `change_building_temperature(handle, temperature)`：校验句柄 → 写记录温度 + temperatureInfo[handleIndex]（原版 L136853-136909）。
- 事件句柄用建筑句柄本身（原版 GetSimHandleForBuildingHandle 等价）。

### 3.3 `update_building_to_building`（原版 L138358-138640）

逐记录（区域 = 整图，行为等价）：

1. `record.heatExchange_handle` 查建筑数据（temperature、totalHC、TC、overheat）；遍历接触列表求 min/max_seen；
2. 每接触（8B {contact_handle, cellsInContact}）：
   - 接触建筑数据（temperature、totalHC、TC、overheat）；
   - `f21 = contact.totalHC`；若 `contactT < buildingT` → `f21 = building.totalHC`（暖侧 HC）；
   - `k = dt × buildingToBuildingTemperatureScale × contactTC × buildingTC × (contactT − buildingT) × f21 × 0.005`；
   - `new_building = k/buildingHC + buildingT` 钳制 [min_seen, max_seen]；
   - `new_contact = contactT − k/contactHC` 钳制 [min_seen, max_seen]；
   - 能量守恒：`min(|Δbuilding|×buildingHC, |Δcontact|×contactHC)` 带符号 → 重算 new_contact/new_building；
   - 交叉（方向反转）→ 热容加权平均钳制；
   - 事件（contact 侧）：`contact.overheat ≤ new_contact` → buildingOverheatInfo(contact 句柄)；
     `contact.overheat ≤ old_contactT ∧ new_contact < overheat` → buildingNoLongerOverheatedInfo(contact 句柄)；
   - 写回：`contact.temperature = new_contact`、`building.temperature = new_building`
     （change_building_temperature，同时刷新 temperatureInfo）；
3. 记录数据更新。

### 3.4 集成

- frame_manager：Register→adds 8B、RemoveInContact→modifies 8B、AddInContact→add_building_in_contact **12B**（修正）、Remove→removes 8B；
- frame_processor：`process_building_to_building_messages`（adds→modifies→addInContact→removes，对照 L116540-116650）；
- update_data 组件阶段：`update_building_to_building(sim_data, 0.2)`（与建筑↔格子/碎片并列，原版 UpdateComponents L145434 同一循环）；
- copy_sim_data_to_game：无新增输出（事件走既有 buildingOverheatInfo/buildingNoLongerOverheatedInfo）。

## 4. 测试

单测（LIB_TESTS_LOCK 串行）：

- Register/Add/RemoveContact/Unregister（句柄、接触列表增删、回调）；
- Update 换热：公式数值、能量守恒、钳制、交叉、暖侧 HC 选择；
- 过热事件（contact 侧 =、<、> 边界）；
- 消息路由（frame_manager 尺寸 8/8/12/8）；
- 回归：全量 `cargo test -- --test-threads=1` + Release。

## 5. 验收

- 全量测试通过 + Release 构建成功。
- 用户实测：导热板中心格在真空与任何事物换热；出入口与方块换热；两导热板相邻互换热；温度收敛与原版一致。

## 6. 范围外（记录）

- 变温板（ThermalBlock）：纯 C#，不做；
- 接触列表的计算（C# 侧）；导热板对管道/电网的特殊交互（C# ConduitFlow）；
- record@0 写入点的反汇编细节留实现期核对（不影响语义）。
