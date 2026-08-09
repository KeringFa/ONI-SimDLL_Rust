# 方向 1：1 星 1 线程（超线程优化）运行时多模式调度器 — 设计规格

日期：2026-08-09
状态：待用户审查

## 1. 背景与目标

- d3 方向 2（rayon 装箱，线程数 = 物理核 − 1）已实现并实测有效（多星卡顿改善、FPS 提升）。
- 方向 1（每世界一线程，利用超线程逻辑核）用户实测可再提升约 +5 FPS，但超线程使用有争抢风险，
  故做成**玩家可选、默认关闭**的优化。
- 目标：把当前“编译期二选一（串行/方向 2）”改为**运行时多模式调度器**：
  串行（单区域/单星）→ 方向 2（rayon 装箱）→ 方向 1（每世界线程 + 亲和性）。

## 2. 约束（用户决策汇总）

1. 单区域/单星永远走串行（行为零变化，与原版一致）。
2. 方向 1 默认关闭；接口 mod 新增“启用超线程优化”勾选项（勾选项 2）+ 对应提示（提示 2）。
3. C# 端启动时检测 CPU 超线程：无超线程 → 勾选项 2 灰显禁用；检测失败 → 保守禁用。
4. 世界大小一律取 **Remix 之后**的实际值（active_regions / worldPlacements），不用静态星群模板；
   变体星固定 8192 格（MixingCeresAsteroid 64×128），至多 3 个变体（Ceres/Marinea/Relica）。
5. 太空员舱集群（HabitatModuleMedium/Small，各 32×32=1024 格）**始终合并为一个线程**，
   动态增删世界时重新分组（10 个也仅 10240 格）。
6. 剩余最小世界**两两合并**（变体后总 tiles 上限 19584 格 = MiniBadlandsStart）。
7. 大星球之间**绝不共享物理核**；超线程线程只与“负载最低的核”共享
   （最小大世界组的核）。注意：**无法绑定/利用 Unity 主线程所在的物理核**
   （那需要 SetProcessAffinityMask 影响全进程，太激进），故超线程线程只
   放在本 DLL 独占的物理核组的空闲逻辑核上。
8. 配置改动需重启生效（与日志开关一致，提示 3 语义）。

## 3. 架构设计

### 3.1 调度器模式

```text
enum SchedulerMode { Auto, D2, D1 }
```

- 全局模式由接口 mod 经新 FFI `RS_SetSchedulerMode` 设置（DLL 启动时，OnLoad 之后、SIM_Initialize 之前）。
- `update_data` 每帧按模式分发：
  - `Auto`（默认）：区域数 > 1 → D2，否则串行（即当前行为）；
  - `D2`：强制方向 2（rayon 装箱，现有管线）；
  - `D1`：方向 1（本节以下），启动时校验超线程可用，失败自动回退 D2。

### 3.2 方向 1 分组与线程模型

**分组算法（affinity 模块输入：Remix 后世界 tiles 列表 + 太空员舱集群）**：

1. 世界按 tiles 降序排序（太空员舱集群按合计 tiles 参与排序）；
2. 物理核组：取前 `P = 物理核数 − 1` 组，每组独占一个物理核（各用 1 个逻辑核）；
3. 超线程组：其余组按 tiles 两两合并（最小两个合并、次小两个合并……），
   加太空员舱集群，共 `H` 组；
4. 亲和性分配：
   - 物理核组 → 物理核 1..P（亲和掩码为对应逻辑核）；
   - 超线程组 → 优先放“最小物理核组（负载最低的大世界）”的空闲逻辑核，
     让争抢集中在负载最低的核上；不触碰 Unity 主线程的核。
5. 全部核逻辑核数 == 1（无 HT）→ D1 不可用，回退 D2。

**线程模型（不用 rayon）**：每组一个 `std::thread` + 每线程一个 mpsc channel。
帧循环：主线程按组投递“模拟一帧”任务 → 各线程跑该组的
`run_region_physics_a(parallel=false)` + 事件回写（region sink）→ 帧尾同步合并。
理由：rayon 工作窃取会破坏亲和性，固定线程 + 固定投递才能保证“世界的物理只在指定核上跑”。

### 3.3 新模块（Rust 侧）

- `src/d3_rayon/affinity.rs`：枚举 CPU 拓扑（GetLogicalProcessorInformation）、
  逻辑核索引、亲和掩码、超线程可用性检测；
- `src/d3_rayon/d1_pipeline.rs`：方向 1 管线（分组 → 线程组执行器 → 帧同步 → 事件合并）；
- `src/d3_rayon/scheduler.rs`：模式状态 + `RS_SetSchedulerMode` FFI + 每帧分发；
- 现有 `bin_pack.rs`（LPT 装箱）与 `pipeline.rs`（D2）保持不变。

### 3.4 接口 mod（C# 侧）

- `CpuInfo.cs`：P/Invoke `GetLogicalProcessorInformation` → 物理核数 / 每核逻辑核数 /
  `HasHyperThreading()`；失败返回 false（保守禁用）；
- `Options.cs`：新增勾选项 2（“启用超线程优化”）+ 提示 2；
  构建配置屏时检测，无 HT → 勾选项 2 灰显禁用（`toggle.enabled = false` + 视觉降 alpha）；
- `ModConfig.cs`：新增 `HyperThreadOptimizationEnabled`（默认 false），config.json 持久化；
- `Loader.OnLoad`：读取配置 → 按模式调用 `RS_SetSchedulerMode`（含“检测失败强制 D2”）。

## 4. 数据流

```text
玩家勾选“启用超线程优化” → OK（保存 config.json，弹重启提示，不立即生效）
→ 重启 → 接口 mod OnLoad：
    CpuInfo.HasHyperThreading()？否 → 灰显（本次仍 D2）
    是 → ModConfig.Apply → RS_SetSchedulerMode(D1)
→ Rust 调度器：D1 启动时校验超线程 → 分组 → 绑定亲和性 → 每帧按组并行
→ 太空员舱增删/新世界激活 → active_regions 变化 → 重新分组
```

## 5. 错误处理与降级

| 场景 | 行为 |
|---|---|
| C# 检测无超线程 / 检测失败 | 勾选项 2 灰显；不写入 config；DLL 走 D2 |
| Rust 收到 D1 但拓扑校验失败 | 回退 D2，日志告警 |
| 线程创建失败 | 回退 D2 |
| 配置启用 D1 后重启 | 重启时重新校验拓扑（配置与拓扑一致才生效） |

## 6. 测试

1. **分配算法单元测试**：模拟 8C16T + 11 星（5 同大小 + 6 大小不一）+ 变体（8192）+
   太空员舱集群（0/5/10 个），断言：大世界组互不共享物理核、超线程组落在指定核、总数不超逻辑核。
2. **拓扑检测测试**：本机实际枚举结果 sanity（物理核数 ≥ 1、逻辑核数 ≥ 物理核数）。
3. **调度一致性测试**：同一多区域布局分别走串行 / D2 / D1，质量守恒一致（沿用现有模式）。
4. **C# 勾选项测试**：本机结果（有 HT → 可勾选；模拟无 HT → 灰显）手工验证。

## 7. 里程碑

1. `affinity.rs`（拓扑枚举 + 分配算法）+ 单元测试；
2. `d1_pipeline.rs`（线程组执行器 + channel + 帧同步）；
3. `scheduler.rs`（模式 + `RS_SetSchedulerMode` + 分发接线）；
4. 接口 mod：`CpuInfo.cs` + 勾选项 2 + 灰显 + config 字段；
5. 联调 + 用户实测（含太空员舱增删、变体星）。

## 8. 明确不做（YAGNI）

- 不做方向 1 与方向 2 的运行时热切换（配置改动一律重启生效）；
- 不做每物理核多线程的负载均衡（工作窃取）——方向 1 固定绑定；
- 不做非 Windows 平台支持（ONI 仅 Windows）。
