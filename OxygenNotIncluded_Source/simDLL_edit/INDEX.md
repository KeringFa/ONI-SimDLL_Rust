# simDLL_source 分段索引

> 自动生成于 2026-07-29 18:53:28 by split_source.ps1
> 源文件: \SimDLL_Source.c\ (268587 行)

## 文件清单

| 文件 | 行号范围 | 函数数 | 写入行数 | 状态 | 对应 Rust 模块 |
|---|---|---|---|---|---|
| 00_types_reference.c | 1-93298 | 0 | 93298 | 参考 | (参考，不重写) |
| 01_sim_api.c | 98220-265335 | 23 | 15203 | Phase 0 重写 | sim_api.rs |
| 02_save_load.c | 96334-98035 | 13 | 1096 | Phase 3 ⭐ | sim.rs / sim_frame_manager.rs |
| 03_elements.c | 96793-126392 | 5 | 566 | Phase 2 | (Phase 2) |
| 04_temperature.c | 133190-148812 | 6 | 3007 | Phase 7 ⭐ | (Phase 7) |
| 05_liquid_flow.c | 145676-149874 | 10 | 2896 | Phase 8 | (Phase 8) |
| 06_process_messages.c | 115412-245826 | 12 | 1799 | Phase 6 | message_handler.rs (Phase 6) |
| 07_conduit_temperature.c | 96182-250618 | 15 | 649 | Phase 9 | (Phase 9) |
| 08_sim_frame_manager.c | 113784-132750 | 11 | 2165 | Phase 6 | sim_frame_manager.rs |
| 09_crashdump_logger.c | 98741-150562 | 10 | 1069 | Phase 0 重写 | logger.rs / crashdump.rs / frame_sync.rs |
| 10_kprofiler.c | 168664-265716 | 55 | 1426 | Phase 10 可选 | (Phase 10 可选) |
| 11_msvcrt_ignored.c | 93299-268587 | 1828 | 145413 | 参考 | (参考，用 Rust 标准库替代) |

## 函数清单（按文件分组，前 20 个标记）

### 01_sim_api.c
- L98220  SIM_BeginSave
- L98242  SIM_BeginSave
- L98388  SIM_DebugCrash
- L98393  SIM_DebugCrash
- L98408  SIM_EndSave
- L98411  SIM_EndSave
- L98421  SIM_HandleMessage
- L98431  SIM_HandleMessage
- L98461  SIM_HandleMessages
- L98475  SIM_HandleMessages
- L98515  SIM_Initialize
- L98527  SIM_Initialize
- L98562  SIM_Shutdown
- L98567  SIM_Shutdown
- L98583  SYSINFO_Acquire
- L98591  SYSINFO_Acquire
- L98612  SYSINFO_Release
- L98617  SYSINFO_Release
- L250619  SIM_BeginSave_dtor_1
- L250628  SIM_HandleMessage_dtor_0
- ... (共 23 个)

### 02_save_load.c
- L96334  Load
- L96538  Start
- L96586  AllocateCells
- L96624  DefineWorldOffsets
- L96673  PrepareGameData
- L96732  ClearUnoccupiedCells
- L96813  SetSavedOptions
- L96832  ResizeAndInitializeVacuumCells
- L96841  CreateDiseaseTable
- L97330  CellRead
- L97430  CleanUp
- L97599  GameSyncFunction
- L97621  PrepareGameDataUpdate

### 03_elements.c
- L96793  CreateElementInteractionsLocked
- L119291  CreateElementsTable
- L119697  DestroyElementsTable
- L119723  GetElementIndex
- L126337  CreateElementInteractions

### 04_temperature.c
- L133190  DoLoadTimeStateTransition
- L133737  UpdateTemperature
- L134004  UpdateTemperatureForBackwallExchange
- L140988  CalculateCombinedTemperature
- L147904  DoPartialHeatTransition
- L148270  DoPartialMelt

### 05_liquid_flow.c
- L145676  UpdateExposedToSunPropertyTexture
- L145899  UpdateFlowTexture
- L145978  UpdateLiquidPropertyTexture
- L146566  IsLiquidPermeable
- L146594  IsSolid
- L146719  UpdateLiquid
- L148813  DoUnstableCheckBasic
- L148909  DoUnstableCheckWithDiagonals
- L149166  Evaporate
- L149292  PostProcessCell

### 06_process_messages.c
- L115412  ProcessBuildingHeatExchangeMessages
- L115539  ProcessCellEnergyModifications
- L115626  ProcessCellProperties
- L115728  ProcessCellRadiationChanges
- L115827  ProcessConsumeDisease
- L115929  ProcessDigPoints
- L116153  ProcessElementChunkMessages
- L117050  ProcessMassConsumption
- L117172  ProcessMassEmission
- L134789  ProcessCellModifications
- L245236  ProcessCodePage
- L245750  ProcessCodePage

### 07_conduit_temperature.c
- L96182  ConduitTemperatureManager_Add
- L96191  ConduitTemperatureManager_Clear
- L96198  ConduitTemperatureManager_Clear
- L96219  ConduitTemperatureManager_Initialize
- L96226  ConduitTemperatureManager_Initialize
- L96244  ConduitTemperatureManager_Remove
- L96258  ConduitTemperatureManager_Remove
- L96291  ConduitTemperatureManager_Set
- L96294  ConduitTemperatureManager_Set
- L96303  ConduitTemperatureManager_Shutdown
- L96308  ConduitTemperatureManager_Shutdown
- L96321  ConduitTemperatureManager_Update
- L96326  ConduitTemperatureManager_Update
- L250122  ConduitTemperatureManager_Initialize_dtor_0
- L250131  ConduitTemperatureManager_Remove_dtor_0

### 08_sim_frame_manager.c
- L113784  SimFrameManager::SimFrameManager
- L114310  SimFrameManager::~SimFrameManager
- L114416  SimFrameManager::BeginFrameProcessing
- L114481  SimFrameManager::EndFrameProcessing
- L115350  SimFrameManager::NewFrame
- L116325  SimFrameManager::ProcessFrame
- L117382  SimFrameManager::ProcessNextFrame
- L130556  Sim::Sim
- L130946  Sim::_vector_deleting_destructor__adjustor_72__
- L130999  Sim::_scalar_deleting_destructor_
- L132665  Sim::Main

### 09_crashdump_logger.c
- L98741  InitializeCrashDumpHandler
- L98854  SimDLLUnhandledExceptionFilter
- L120320  KCrashReporterReportMessage
- L127147  FrameSync::GameSync
- L127240  FrameSync::SimSync
- L127279  FrameSync::clear
- L149957  Logger::Logger
- L149995  Logger::_scalar_deleting_destructor_
- L150024  Logger::Log
- L150551  Timer::Initialize

### 10_kprofiler.c
- L168664  kprofiler_start_profiling
- L181832  kprofile_record_string
- L181853  kprofile_record_string
- L181909  kprofiler_begin_section
- L181941  kprofiler_begin_section
- L182009  kprofiler_counter
- L182037  kprofiler_counter
- L182101  kprofiler_end_section
- L182123  kprofiler_end_section
- L182188  kprofiler_flush_data_sender
- L182193  kprofiler_flush_data_sender
- L182216  kprofiler_get_thread_uid
- L182224  kprofiler_get_thread_uid
- L182236  kprofiler_load_plugin
- L182242  kprofiler_load_plugin
- L182263  kprofiler_ping
- L182294  kprofiler_ping
- L182362  kprofiler_set_thread_info
- L182374  kprofiler_set_thread_info
- L182415  kprofiler_start_file_data_sender
- ... (共 55 个)

### 11_msvcrt_ignored.c
- L93299  thread_start<unsigned_int_
- L93481  _dynamic_initializer_for__gConduitTemperatureManager__
- L93490  _dynamic_initializer_for__QUERIED_SYSTEM_INFO__
- L93499  _dynamic_initializer_for__gFrameSync__
- L93515  _dynamic_initializer_for__gSim__
- L93524  _dynamic_initializer_for__gElementIndices__
- L93554  _dynamic_initializer_for__gElementLightAbsorptionData__
- L93570  _dynamic_initializer_for__gElementLiquidData__
- L93586  _dynamic_initializer_for__gElementNames__
- L93602  _dynamic_initializer_for__gElementPhysicsData__
- L93618  _dynamic_initializer_for__gElementPostProcessData__
- L93634  _dynamic_initializer_for__gElementPressureData__
- L93650  _dynamic_initializer_for__gElementPropertyTextureData__
- L93666  _dynamic_initializer_for__gElementRadiationData__
- L93682  _dynamic_initializer_for__gElementStateData__
- L93698  _dynamic_initializer_for__gElementTemperatureData__
- L93714  _dynamic_initializer_for__gElements__
- L93730  _dynamic_initializer_for__gSimData__
- L93739  _dynamic_initializer_for__gDisease__
- L93748  _dynamic_initializer_for__gGasObliterations__
- ... (共 1828 个)

