using System;
using System.IO;
using Newtonsoft.Json;
using UnityEngine;

namespace SimDLL_Rust
{
    /// <summary>
    /// mod 配置：config.json（位于 mod 目录）。当前管理 Rust simDLL 的日志开关。
    /// 默认不启用日志（普通玩家用不到；发布前再讨论哪些日志应被输出），
    /// 日志级别仍可由 config.json 手动指定（LogEnabled=true 时生效）。
    /// 保存/应用时机：Options 对话框确认后立即保存并调用 RS_SetLogEnabled 应用。
    /// </summary>
    public static class ModConfig
    {
        private const string CONFIG_FILE = "config.json";
        private static string modPath;

        public static bool Dirty { get; private set; }

        /// 是否写日志（false → 级别强制 off）。
        public static bool LogEnabled { get; private set; } = false;

        /// 启用超线程优化（方向 1：1 星 1 线程）。默认 false；无超线程 CPU 不可用。
        public static bool HyperThreadOptimizationEnabled { get; private set; } = false;

        /// 预留物理线程数（1~4，默认 1）：留给游戏主线程/系统，D1/D2 并行路径通用。
        public static int ReservedCores { get; private set; } = DEFAULT_RESERVED_CORES;

        public const int DEFAULT_RESERVED_CORES = 1;
        public const int MAX_RESERVED_CORES = 4;

        /// 启用 D4 切片并行（实验性）。默认 false（与 Rust 侧默认 true 不同——
        /// Rust 侧默认值是接口 mod 未适配时的兜底；C# 侧接管后以玩家配置为准）。
        public static bool SliceEnabled { get; private set; } = false;

        /// D4 切片边长（8~64，默认 32）。
        public static int SliceSize { get; private set; } = DEFAULT_SLICE_SIZE;

        public const int DEFAULT_SLICE_SIZE = 32;
        public const int MIN_SLICE_SIZE = 8;
        public const int MAX_SLICE_SIZE = 64;

        public static void Load(string path)
        {
            modPath = path;

            try
            {
                string configPath = Path.Combine(path ?? "", CONFIG_FILE);
                if (File.Exists(configPath))
                {
                    var data = JsonConvert.DeserializeObject<ConfigData>(File.ReadAllText(configPath));
                    if (data != null)
                    {
                        LogEnabled = data.LogEnabled;
                        HyperThreadOptimizationEnabled = data.HyperThreadOptimizationEnabled;
                        ReservedCores = ClampReservedCores(data.ReservedCores);
                        SliceEnabled = data.SliceEnabled;
                        SliceSize = ClampSliceSize(data.SliceSize);
                    }
                }
            }
            catch
            {
                LogEnabled = true;
                HyperThreadOptimizationEnabled = false;
                ReservedCores = DEFAULT_RESERVED_CORES;
                SliceEnabled = false;
                SliceSize = DEFAULT_SLICE_SIZE;
            }
        }

        public static void SetLogEnabled(bool value)
        {
            if (LogEnabled != value)
            {
                LogEnabled = value;
                Dirty = true;
            }
        }

        public static void SetHyperThreadOptimizationEnabled(bool value)
        {
            if (HyperThreadOptimizationEnabled != value)
            {
                HyperThreadOptimizationEnabled = value;
                Dirty = true;
            }
        }

        public static void SetReservedCores(int value)
        {
            int clamped = ClampReservedCores(value);
            if (ReservedCores != clamped)
            {
                ReservedCores = clamped;
                Dirty = true;
            }
        }

        public static int ClampReservedCores(int value)
            => Math.Min(Math.Max(value, DEFAULT_RESERVED_CORES), MAX_RESERVED_CORES);

        public static void SetSliceEnabled(bool value)
        {
            if (SliceEnabled != value)
            {
                SliceEnabled = value;
                Dirty = true;
            }
        }

        public static void SetSliceSize(int value)
        {
            int clamped = ClampSliceSize(value);
            if (SliceSize != clamped)
            {
                SliceSize = clamped;
                Dirty = true;
            }
        }

        public static int ClampSliceSize(int value)
            => Math.Min(Math.Max(value, MIN_SLICE_SIZE), MAX_SLICE_SIZE);

        /// 应用当前配置到 Rust（日志 / 预留核 / D4 切片开关与尺寸）。
        public static void Apply()
        {
            Native.RS_SetLogEnabled(LogEnabled);
            Native.RS_SetReservedCores(ReservedCores);
            Native.RS_SetSliceEnabled(SliceEnabled);
            Native.RS_SetSliceSize(SliceSize);
        }

        public static void Save()
        {
            if (!Dirty) return;

            try
            {
                string configPath = Path.Combine(modPath ?? "", CONFIG_FILE);
                var data = new ConfigData
                {
                    LogEnabled = LogEnabled,
                    HyperThreadOptimizationEnabled = HyperThreadOptimizationEnabled,
                    ReservedCores = ReservedCores,
                    SliceEnabled = SliceEnabled,
                    SliceSize = SliceSize,
                };
                File.WriteAllText(configPath, JsonConvert.SerializeObject(data, Formatting.Indented));
                Dirty = false;
            }
            catch (Exception e)
            {
                Debug.LogError("[SimDLL_Rust] 配置保存失败: " + e.Message);
            }
        }

        private class ConfigData
        {
            public bool LogEnabled { get; set; } = false;
            public bool HyperThreadOptimizationEnabled { get; set; } = false;
            public int ReservedCores { get; set; } = DEFAULT_RESERVED_CORES;
            public bool SliceEnabled { get; set; } = false;
            public int SliceSize { get; set; } = DEFAULT_SLICE_SIZE;
        }
    }
}
