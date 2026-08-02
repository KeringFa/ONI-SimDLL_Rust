# SDD 进度账本 — 新游戏生成（InitializeFromCells）

计划：docs/superpowers/plans/2026-08-01-new-game-generation.md
规格：docs/superpowers/specs/2026-08-01-new-game-generation-design.md
项目约束：不执行 git 操作；不部署 DLL；审查无 git diff 可用 → 审查者读源文件区段。

## 任务状态

- Task 1: complete (buffer.rs remaining + peek；测试 16/16 + 全库 190/190；review clean：规格✅ 质量通过)
- Task 2: complete (sim_data.rs new_for_allocate 补缺 L528-540 + 测试 L702-716；12/12 + 全库 191/191；review clean：规格✅ 质量通过)
- Task 3: complete (sim_data.rs initialize_boundary L583-629 + 测试 L766-804；13/13 + 全库 192/192；review clean：规格✅ 质量通过)
- Task 4: complete (sim_data.rs settle_thermal_boundaries L631-672 + 测试 L849-884；14/14 + 全库 193/193；review clean：规格✅ 质量通过。注：简报测试期望值 3400/7 有误，实现者修正为 3100/6，审查独立核算确认实现者正确，计划文档已同步修正)
- Task 5: complete (save_load.rs handle_initialize_from_cells L1065-1186 + 测试/辅助 L1790-1949；21/21 + 全库 197/197；review clean：规格✅ 质量通过。注：简报 `(*G_SIM_DATA).0` 写法有误（Mutex），实现者适配为 lock 模式+LIB_TESTS_LOCK，审查核实无死锁路径，计划文档已同步修正)
- Task 6: complete (串行全量 197 lib + 49 集成全过；release 编译成功 SimDLL.dll 1.12MB；代办.txt 已更新为"待实测"并归档遗留备忘。注：全量并行测试曾 1 次 lib 29 失败，6 次复跑全过，已记代办遗留)
- 最终宽范围审查: complete（reasoning 模型）——**可交付实测**，无关键/重要阻塞项；1 项重要建议（Start 旧存档 settle 一行补齐）已记代办待用户裁决；10 项次要发现分诊全部为"留待后续/无需处理"

## 发现记录（待最终审查分诊/用户裁决）

- [重要·计划强制] buffer.rs:139/148/158/168 peek 方法 `offset + N > buffer_size` 在 offset 接近 u64::MAX 时溢出 → panic 而非 Err(EndOfBuffer)。无内存不安全（切片索引自带检查），与既有 read_byte/read_bytes/skip 同模式。简报逐字强制该写法；实际调用方（handler）传入经校验的内部 offset，不可达。候选修法：`offset.checked_add(N).map_or(true, |end| end > self.buffer_size)`。→ 待用户裁决：改规格统一修，或保持与全库既有模式一致。
- [次要] 4 个 peek 方法各重复"检查+unsafe 建切片"5 行；既有 read_ushort/read_int 示范委托 read_bytes 的 DRY 写法。
- [次要] 测试未覆盖空 buffer 的 read_u8_at(0) 报错与 remaining() saturate 路径。
- [次要·计划强制] 任务 2 索引断言对哈希转录错误不敏感（空元素表下任何哈希都返回 65535，哈希写错测试仍过）；正确性靠人工逐字核对（任务 2 审查已核对通过）。
- [次要·计划强制] 任务 3 裸指针借用模式（backwall 贯穿 + cells/updated 每轮重新解引用）：审查确认三块内存互不相交、无别名、无 UB。
- [次要] initialize_boundary 退化尺寸（height=1 顶底行重叠、width=1 左右列重合）行为未定义；规格未要求，真实世界尺寸不触及。
- [次要] settle_thermal_boundaries 的 count==0 路径（邻居全排除时保留原温度）无测试覆盖；简报局限，测试环境难构造独立 void 邻居。
- [次要] 任务 5 报告措辞小误：称 settle "不取全局锁"（实际经 get_element_index_pub 取 G_ELEMENTS_TABLE）；无死锁结论不变（锁序单向）。
- [次要·计划强制] test 3 覆盖缺口：截断拒绝只断言"gSimData 保持 null"，未覆盖"已存在旧实例时拒绝后旧实例保留"分支（代码路径上校验在锁之前 return，风险仅回归保护缺失）。→ 最终审查分诊是否补强。
- [次要] handler `num_cells * 40` 在 width/height 接近 i32::MAX 时理论溢出（现实消息不可达，原版完全无校验）。
