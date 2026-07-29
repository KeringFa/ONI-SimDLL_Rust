<#
.SYNOPSIS
  按设计文档 §1 将 SimDLL_Source.c 切分为 10+2 个文件。
.DESCRIPTION
  扫描 Ghidra 函数标记 "// 0xADDR  N  FuncName" 和函数定义行，
  按函数名前缀归类到 12 个文件。脚本不修改原文件。
  调用方式：powershell -ExecutionPolicy Bypass -File split_source.ps1
#>
param(
    [string]$Source = "..\SimDLL_Source.c",
    [string]$OutDir = "."
)

$ErrorActionPreference = 'Stop'
Set-Location -Path $PSScriptRoot

if (-not (Test-Path $Source)) {
    throw "Source file not found: $Source"
}

# 读取全部行（268587 行，约 8.6MB，可一次读入）
$lines = Get-Content -Path $Source -Encoding UTF8
$totalLines = $lines.Count
Write-Host "Source has $totalLines lines"

# 1. 识别函数标记行：匹配 "                    // 0x4770  1  ConduitTemperatureManager_Add"
#    格式：前导空格 + "// 0x" + hex + 空白 + 数字 + 空白 + 函数名
$markerRe = [regex]'^\s*//\s+0x[0-9a-fA-F]+\s+\d+\s+(\w+)'

# 2. 识别函数定义行：返回类型 + [calling convention] + 函数名 + (
#    两种形式：
#    a) 带调用约定：  RETTYPE [*] __cdecl/__thiscall/... [*] FuncName(
#       RETTYPE 可为原始类型（void/int/...）或自定义类型（Handle/basic_string<>/T_HRESULT/...）
#    b) 不带调用约定：PRIMITIVE_TYPE [*] FuncName(   （如 _dtor_ 函数）
#    capture group 1 = 函数名（支持 C++ 名称：std::vector<>::_Resize<>, ~CompactedVector<> 等）
$typeRe = [regex]'^\s*(?:[\w:<>]+\s+(?:\*\s*)?(?:__cdecl|__thiscall|__fastcall|__stdcall|__vectorcall)\s+(?:\*\s*)?|(?:void|int|bool|undefined\d?|dword|qword|longlong|long|uint|ulong|ulong64|char|float|double|ushort|short|byte|schar|wchar16|wchar32|struct\s+\w+|enum\s+\w+)\s+(?:\*\s*)?)((?:[\w:$<>~]+))\s*\('

# 收集所有函数标记：list of [PSCustomObject]@{ LineNo; FuncName; }
# 同时匹配 $markerRe（Ghidra 标记）和 $typeRe（函数定义行），按行号去重
$markers = New-Object System.Collections.Generic.List[object]
for ($i = 0; $i -lt $lines.Count; $i++) {
    $m = $markerRe.Match($lines[$i])
    if ($m.Success) {
        $markers.Add([PSCustomObject]@{
            LineNo   = $i + 1          # 1-based
            FuncName = $m.Groups[1].Value
        })
        continue  # 同一行不重复匹配
    }
    $t = $typeRe.Match($lines[$i])
    if ($t.Success) {
        $markers.Add([PSCustomObject]@{
            LineNo   = $i + 1
            FuncName = $t.Groups[1].Value
        })
    }
}
Write-Host "Found $($markers.Count) function markers"

# 辅助：根据函数名返回目标文件名
function GetTargetFile([string]$name) {
    if ($name -match '^SIM_' -or $name -match '^SYSINFO_') { return '01_sim_api.c' }
    if ($name -match '^ConduitTemperatureManager_') { return '07_conduit_temperature.c' }
    if ($name -match '^kprofiler_' -or $name -match '^kprofile_') { return '10_kprofiler.c' }
    if ($name -match '^InitializeCrashDumpHandler$' -or $name -match '^SimDLLUnhandledExceptionFilter$' -or $name -match '^KCrashReporterReportMessage$' -or $name -match '^Logger::' -or $name -match '^Timer::' -or $name -match '^FrameSync::') { return '09_crashdump_logger.c' }
    if ($name -match '^UpdateTemperature' -or $name -match '^CalculateCombinedTemperature' -or $name -match '^DoPartialHeatTransition' -or $name -match '^DoPartialMelt' -or $name -match '^DoLoadTimeStateTransition') { return '04_temperature.c' }
    if ($name -match '^UpdateLiquid' -or $name -match '^UpdateFlowTexture' -or $name -match '^UpdateLiquidPropertyTexture' -or $name -match '^UpdateExposedToSunPropertyTexture' -or $name -match '^IsLiquidPermeable' -or $name -match '^IsSolid' -or $name -match '^Evaporate' -or $name -match '^PostProcessCell' -or $name -match '^DoUnstableCheck') { return '05_liquid_flow.c' }
    if ($name -match '^Process') { return '06_process_messages.c' }
    if ($name -match '^CreateElementsTable$' -or $name -match '^DestroyElementsTable$' -or $name -match '^GetElementIndex$' -or $name -match '^CreateElementInteractions') { return '03_elements.c' }
    if ($name -match '^(Load|Start|AllocateCells|DefineWorldOffsets|PrepareGameData|ClearUnoccupiedCells|CreateElementInteractionsLocked|SetSavedOptions|ResizeAndInitializeVacuumCells|CreateDiseaseTable|CellRead|CleanUp|GameSyncFunction|PrepareGameDataUpdate)$') { return '02_save_load.c' }
    if ($name -match '^Sim::' -or $name -match '^SimFrameManager::') { return '08_sim_frame_manager.c' }
    return '11_msvcrt_ignored.c'
}

# 计算每个函数的"管辖区间"：从标记行到下一个标记行前一行
# 第一个标记前的所有内容（含 typedef/struct）归 00_types_reference.c
# 没有标记的函数会被归入 11_msvcrt_ignored.c（通过 catch-all 兜底）

# 收集每个目标文件的行号区间列表
$fileRanges = @{
    '00_types_reference.c'      = @()
    '01_sim_api.c'              = @()
    '02_save_load.c'            = @()
    '03_elements.c'             = @()
    '04_temperature.c'          = @()
    '05_liquid_flow.c'          = @()
    '06_process_messages.c'     = @()
    '07_conduit_temperature.c'  = @()
    '08_sim_frame_manager.c'    = @()
    '09_crashdump_logger.c'     = @()
    '10_kprofiler.c'            = @()
    '11_msvcrt_ignored.c'       = @()
}
$funcCount = @{}
foreach ($k in $fileRanges.Keys) { $funcCount[$k] = 0 }

# 第一个标记前的内容 -> 00_types_reference.c
if ($markers.Count -gt 0) {
    $firstMarkerLine = $markers[0].LineNo
    # 从 1 到 firstMarkerLine-1（1-based）
    $fileRanges['00_types_reference.c'] += ,@{ Start = 1; End = $firstMarkerLine - 1 }
}

# 每个函数区间 -> 对应文件
for ($i = 0; $i -lt $markers.Count; $i++) {
    $start = $markers[$i].LineNo
    $end = if ($i -lt $markers.Count - 1) { $markers[$i + 1].LineNo - 1 } else { $totalLines }
    $target = GetTargetFile $markers[$i].FuncName
    $fileRanges[$target] += ,@{ Start = $start; End = $end }
    $funcCount[$target]++
}

# 11_msvcrt_ignored.c 无兜底：仅由 catch-all GetTargetFile 填充。
# 若没有函数归到这里（理论上不可能，因为 catch-all 涵盖所有未匹配函数），
# 文件就只有文件头——这是合法的。移除原兜底逻辑避免与 10_kprofiler.c 区间重叠。

# 输出每个文件
$indexEntries = @()
foreach ($file in @('00_types_reference.c','01_sim_api.c','02_save_load.c','03_elements.c','04_temperature.c','05_liquid_flow.c','06_process_messages.c','07_conduit_temperature.c','08_sim_frame_manager.c','09_crashdump_logger.c','10_kprofiler.c','11_msvcrt_ignored.c')) {
    $ranges = $fileRanges[$file]
    $outPath = Join-Path $OutDir $file
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.AppendLine("// =============================================================")
    [void]$sb.AppendLine("// FILE: $file")
    [void]$sb.AppendLine("// AUTO-GENERATED BY split_source.ps1 - DO NOT EDIT BY HAND")
    [void]$sb.AppendLine("// SOURCE: ../SimDLL_Source.c (268587 lines, Ghidra 12.1.2 decompile)")
    [void]$sb.AppendLine("// =============================================================")
    [void]$sb.AppendLine("")
    $writtenLines = 0
    foreach ($r in $ranges) {
        if ($r.Start -le $r.End -and $r.Start -ge 1 -and $r.End -le $totalLines) {
            $chunk = $lines[($r.Start - 1)..($r.End - 1)]
            foreach ($ln in $chunk) { [void]$sb.AppendLine($ln) }
            $writtenLines += ($r.End - $r.Start + 1)
        }
    }
    [System.IO.File]::WriteAllText($outPath, $sb.ToString(), [System.Text.UTF8Encoding]::new($false))
    $lineSpan = if ($ranges.Count -gt 0) { "$($ranges[0].Start)-$($ranges[$ranges.Count-1].End)" } else { '0' }
    $indexEntries += [PSCustomObject]@{
        File       = $file
        LineRange  = $lineSpan
        FuncCount  = $funcCount[$file]
        WrittenLines = $writtenLines
    }
    Write-Host "  Wrote $file : $writtenLines lines, $($funcCount[$file]) functions, span $lineSpan"
}

# 完整性校验：所有文件写入行数总和 = 源文件总行数 + 头部注释行（每文件 6 行）
$headerLinesPerFile = 7  # 6 注释行 + 1 空行
$expectedTotal = $totalLines + ($headerLinesPerFile * 12)
$actualTotal = ($indexEntries | Measure-Object -Property WrittenLines -Sum).Sum + ($headerLinesPerFile * 12)
Write-Host ""
Write-Host "Completeness check:"
Write-Host "  Source lines: $totalLines"
Write-Host "  Sum of written content lines (excl headers): $(($indexEntries | Measure-Object -Property WrittenLines -Sum).Sum)"
Write-Host "  Headers added: $($headerLinesPerFile * 12) lines across 12 files"
if (($indexEntries | Measure-Object -Property WrittenLines -Sum).Sum -ne $totalLines) {
    Write-Warning "MISMATCH: content lines != source lines (some lines lost or duplicated)"
} else {
    Write-Host "  OK: content lines match source lines"
}

# 生成 INDEX.md
$idx = New-Object System.Text.StringBuilder
[void]$idx.AppendLine("# simDLL_source 分段索引")
[void]$idx.AppendLine("")
[void]$idx.AppendLine("> 自动生成于 $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') by split_source.ps1")
[void]$idx.AppendLine("> 源文件: \`SimDLL_Source.c\` ($totalLines 行)")
[void]$idx.AppendLine("")
[void]$idx.AppendLine("## 文件清单")
[void]$idx.AppendLine("")
[void]$idx.AppendLine("| 文件 | 行号范围 | 函数数 | 写入行数 | 状态 | 对应 Rust 模块 |")
[void]$idx.AppendLine("|---|---|---|---|---|---|")
$rustMap = @{
    '00_types_reference.c'     = '(参考，不重写)'
    '01_sim_api.c'             = 'sim_api.rs'
    '02_save_load.c'           = 'sim.rs / sim_frame_manager.rs'
    '03_elements.c'            = '(Phase 2)'
    '04_temperature.c'         = '(Phase 7)'
    '05_liquid_flow.c'         = '(Phase 8)'
    '06_process_messages.c'    = 'message_handler.rs (Phase 6)'
    '07_conduit_temperature.c' = '(Phase 9)'
    '08_sim_frame_manager.c'   = 'sim_frame_manager.rs'
    '09_crashdump_logger.c'    = 'logger.rs / crashdump.rs / frame_sync.rs'
    '10_kprofiler.c'           = '(Phase 10 可选)'
    '11_msvcrt_ignored.c'      = '(参考，用 Rust 标准库替代)'
}
$statusMap = @{
    '00_types_reference.c'     = '参考'
    '01_sim_api.c'             = 'Phase 0 重写'
    '02_save_load.c'           = 'Phase 3 ⭐'
    '03_elements.c'            = 'Phase 2'
    '04_temperature.c'         = 'Phase 7 ⭐'
    '05_liquid_flow.c'         = 'Phase 8'
    '06_process_messages.c'    = 'Phase 6'
    '07_conduit_temperature.c' = 'Phase 9'
    '08_sim_frame_manager.c'   = 'Phase 6'
    '09_crashdump_logger.c'    = 'Phase 0 重写'
    '10_kprofiler.c'           = 'Phase 10 可选'
    '11_msvcrt_ignored.c'      = '参考'
}
foreach ($e in $indexEntries) {
    [void]$idx.AppendLine("| $($e.File) | $($e.LineRange) | $($e.FuncCount) | $($e.WrittenLines) | $($statusMap[$e.File]) | $($rustMap[$e.File]) |")
}
[void]$idx.AppendLine("")
[void]$idx.AppendLine("## 函数清单（按文件分组，前 20 个标记）")
[void]$idx.AppendLine("")
$grouped = $markers | Group-Object { GetTargetFile $_.FuncName } | Sort-Object Name
foreach ($g in $grouped) {
    [void]$idx.AppendLine("### $($g.Name)")
    foreach ($m in ($g.Group | Select-Object -First 20)) {
        [void]$idx.AppendLine("- L$($m.LineNo)  $($m.FuncName)")
    }
    if ($g.Group.Count -gt 20) {
        [void]$idx.AppendLine("- ... (共 $($g.Group.Count) 个)")
    }
    [void]$idx.AppendLine("")
}
[System.IO.File]::WriteAllText((Join-Path $OutDir 'INDEX.md'), $idx.ToString(), [System.Text.UTF8Encoding]::new($false))
Write-Host "Wrote INDEX.md"
Write-Host "Done."
