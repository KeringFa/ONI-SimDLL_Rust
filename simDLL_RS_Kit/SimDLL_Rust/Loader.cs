using System;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using HarmonyLib;
using KMod;

namespace SimDLL_Rust
{
    /// <summary>
    /// mod 入口：加载原生 Rust simDLL（Windows: SimDLL_RS.sim /
    /// Linux: SimDLL_RS.so，位于本 mod 目录），然后调用 RS_LoaderInstall
    /// 用原生 detour 接管原版 SimDLL 的导出。
    ///
    /// 加载方式说明：游戏经 Assembly-CSharp 的 Sim/ConduitTemperatureManager
    /// extern 直调原版 SimDLL.dll；当前游戏版本的 Harmony 无法 patch extern
    /// （P/Invoke，无 IL 体，实测 U59 抛 InvalidProgramException），因此改用
    /// 原生 detour：RS_LoaderInstall 把原版 16 个导出函数入口改写为绝对跳转，
    /// 重定向到本 mod 的 Rust 实现。
    /// 原版 SimDLL 保持在 Plugins/x86_64 原位，但永远不会被调用。
    ///
    /// 跨平台（2026-09-09）：
    /// - Windows：kernel32 LoadLibrary（.sim 扩展名绕过 KMod.DLLLoader 的托管
    ///   程序集扫描——原生 PE 被当托管加载会抛 BadImageFormatException）。
    /// - Linux：libc dlopen（绝对路径 + RTLD_NOW|RTLD_GLOBAL，dlopen 对扩展名
    ///   无要求；RTLD_GLOBAL 使原版 SimDLL.so 的导出进入全局符号表，与游戏
    ///   extern 绑定行为一致）。
    /// - 其余平台（macOS 等）：优雅降级——跳过原生加载，mod 保持空实现，
    ///   游戏回落原版 sim，不崩溃。
    /// </summary>
    public class Loader : UserMod2
    {
        private static IntPtr nativeModule;

        /// 原生库是否加载并解析成功。false 时 Native 入口全部短路
        /// （如 macOS 兜底、或未来架构不支持的兜底）。
        internal static bool SimAvailable { get; private set; }

        public override void OnLoad(Harmony harmony)
        {
            if (Native.IsWindowsPlatform())
            {
                LoadWindows();
            }
            else if (Environment.OSVersion.Platform == PlatformID.Unix)
            {
                // ONI Linux 版运行于 Mono/Unix；macOS 也是 Unix——dlopen 失败
                // （无 .so）时自然落入优雅降级分支。
                LoadLinux();
            }
            else
            {
                Debug.LogWarning(
                    "[SimDLL_Rust] 本 mod 的原生实现提供 Windows / Linux 版本；当前平台不受支持，已跳过加载（回落原版模拟，游戏正常运行）。");
                SimAvailable = false;
                return;
            }

            if (!SimAvailable)
            {
                return;
            }

            // 安装原生 detour：接管原版 SimDLL 的 16 个导出。
            // 不再调用 base.OnLoad（其 PatchAll 会因 Harmony 无法 patch extern 而崩溃）。
            int installed = Native.RS_LoaderInstall();
            if (installed <= 0)
            {
                throw new InvalidOperationException("SimDLL_Rust: RS_LoaderInstall 失败，安装数=" + installed);
            }

            // 加载 mod 配置并应用到 Rust 日志级别（config.json，Options 屏可改）
            ModConfig.Load(mod?.ContentPath ?? path);
            ModConfig.Apply();

            // 调度模式：默认 Auto(0)——单区域/单星永远走串行（与原版一致），
            // 多区域走 D2（rayon 装箱）；仅当启用超线程优化且 CPU 检测通过 → D1(2)。
            // 注意：不能默认强制 D2(1)，否则单星也会进 D2 并行路径（行为偏离原版串行）。
            int schedulerMode = 0;
            if (CpuInfo.HasHyperThreading() && ModConfig.HyperThreadOptimizationEnabled)
            {
                schedulerMode = 2;
            }
            int appliedMode = Native.RS_SetSchedulerMode(schedulerMode);
            if (schedulerMode == 2 && appliedMode == 1)
            {
                Debug.LogWarning("[SimDLL_Rust] 超线程优化已启用但 CPU 无超线程，回退方向 2");
            }

            // 应用托管补丁（ModsScreen Options 按钮 / Localization 注册）——
            // 注意：不调用 base.OnLoad（其 PatchAll 会扫描并 patch extern，当前
            // Harmony 版本对 extern 生成 wrapper 崩溃）；这里显式 PatchAll 只含
            // 托管方法补丁（extern 补丁类已不存在）。
            harmony.PatchAll(Assembly.GetExecutingAssembly());
        }

        /// Windows 加载：kernel32 LoadLibrary（.sim 绕过 KMod 托管程序集扫描）。
        private void LoadWindows()
        {
            string directory = Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location);
            string nativePath = Path.Combine(directory ?? ".", "SimDLL_RS.sim");
            if (!File.Exists(nativePath))
            {
                throw new FileNotFoundException("SimDLL_Rust: 未找到原生实现 " + nativePath);
            }
            nativeModule = NativeMethods.LoadLibrary(nativePath);
            if (nativeModule == IntPtr.Zero)
            {
                throw new InvalidOperationException(
                    "SimDLL_Rust: LoadLibrary 失败 (" + Marshal.GetLastWin32Error() + "): " + nativePath);
            }
            Native.Resolve(nativeModule);
            SimAvailable = Native.IsAvailable;
        }

        /// Linux 加载：libc dlopen（绝对路径，RTLD_NOW|RTLD_GLOBAL）。
        /// RTLD_GLOBAL：原版 SimDLL.so 由本路径显式加载时，其导出进入全局符号表，
        /// 与游戏 extern 绑定行为一致。
        private void LoadLinux()
        {
            string directory = Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location);
            string nativePath = Path.Combine(directory ?? ".", "SimDLL_RS.so");
            if (!File.Exists(nativePath))
            {
                Debug.LogWarning(
                    "[SimDLL_Rust] 未找到 Linux 原生实现 " + nativePath + "（本包可能只含 Windows 版），已回落原版模拟。");
                SimAvailable = false;
                return;
            }
            // RTLD_NOW=2 | RTLD_GLOBAL=0x100
            nativeModule = NativeMethods.dlopen(nativePath, 2 | 0x100);
            if (nativeModule == IntPtr.Zero)
            {
                Debug.LogError("[SimDLL_Rust] dlopen 失败: " + nativePath + "——已回落原版模拟。");
                SimAvailable = false;
                return;
            }
            Native.Resolve(nativeModule);
            SimAvailable = Native.IsAvailable;
        }

        private static class NativeMethods
        {
            [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
            public static extern IntPtr LoadLibrary(string lpFileName);

            // Linux：libc.so.6 必然已被主进程加载（ONI Linux 版基于 glibc 发行版），
            // dlopen 短名命中已加载副本。
            [DllImport("libc.so.6", SetLastError = true)]
            public static extern IntPtr dlopen(string filename, int flags);
        }
    }
}
