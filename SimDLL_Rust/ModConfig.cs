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
    /// 保存/应用时机：Options 对话框确认后立即保存并调用 RS_SetLogLevel 应用。
    /// </summary>
    public static class ModConfig
    {
        private const string CONFIG_FILE = "config.json";
        private static string modPath;

        public static bool Dirty { get; private set; }

        /// 是否写日志（false → 级别强制 off）。
        public static bool LogEnabled { get; private set; } = false;

        /// 日志级别：off/error/warn/info/debug/trace。
        public static string LogLevel { get; private set; } = DEFAULT_LOG_LEVEL;

        /// 启用超线程优化（方向 1：1 星 1 线程）。默认 false；无超线程 CPU 不可用。
        public static bool HyperThreadOptimizationEnabled { get; private set; } = false;

        public const string DEFAULT_LOG_LEVEL = "info";
        public static readonly string[] VALID_LOG_LEVELS =
            { "off", "error", "warn", "info", "debug", "trace" };

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
                        LogLevel = NormalizeLevel(data.LogLevel) ?? DEFAULT_LOG_LEVEL;
                        HyperThreadOptimizationEnabled = data.HyperThreadOptimizationEnabled;
                    }
                }
            }
            catch
            {
                LogEnabled = true;
                LogLevel = DEFAULT_LOG_LEVEL;
                HyperThreadOptimizationEnabled = false;
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

        public static void SetLogLevel(string value)
        {
            string normalized = NormalizeLevel(value) ?? DEFAULT_LOG_LEVEL;
            if (LogLevel != normalized)
            {
                LogLevel = normalized;
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

        /// 规范化级别字符串；非法值返回 null。
        private static string NormalizeLevel(string value)
        {
            if (string.IsNullOrWhiteSpace(value)) return null;
            string v = value.Trim().ToLowerInvariant();
            foreach (string valid in VALID_LOG_LEVELS)
            {
                if (v == valid) return v;
            }
            return null;
        }

        /// 应用当前配置到 Rust 日志（调用 RS_SetLogLevel）。
        public static void Apply()
        {
            string effective = LogEnabled ? LogLevel : "off";
            // Rust 侧按 UTF-8 NUL 结尾解析（CStr::from_ptr）
            Native.RS_SetLogLevel(System.Text.Encoding.UTF8.GetBytes(effective + "\0"));
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
                    LogLevel = LogLevel,
                    HyperThreadOptimizationEnabled = HyperThreadOptimizationEnabled,
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
            public bool LogEnabled { get; set; } = true;
            public string LogLevel { get; set; } = DEFAULT_LOG_LEVEL;
            public bool HyperThreadOptimizationEnabled { get; set; } = false;
        }
    }
}
