using System;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using HarmonyLib;
using KMod;

namespace SimDLL_Rust
{
    /// <summary>
    /// mod 入口：加载原生 Rust simDLL（SimDLL_RS.sim，位于本 mod 目录），
    /// 然后调用 RS_LoaderInstall 用原生 detour 接管原版 SimDLL.dll 的导出。
    ///
    /// 加载方式说明：游戏经 Assembly-CSharp 的 Sim/ConduitTemperatureManager
    /// extern 直调原版 SimDLL.dll；当前游戏版本的 Harmony 无法 patch extern
    /// （P/Invoke，无 IL 体，实测 U59 抛 InvalidProgramException），因此改用
    /// 原生 detour：RS_LoaderInstall 把原版 16 个导出函数入口改写为绝对跳转，
    /// 重定向到本 mod 的 Rust 实现。
    /// 原版 SimDLL.dll 保持在 Plugins/x86_64 原位，但永远不会被调用。
    /// </summary>
    public class Loader : UserMod2
    {
        private static IntPtr nativeModule;

        public override void OnLoad(Harmony harmony)
        {
            // 显式 LoadLibrary 原生实现（mod 目录），使 DllImport 解析命中。
            // 扩展名必须是 .sim 而非 .dll——ONI 的 KMod.DLLLoader 会把 mod
            // 目录内所有 .dll 当托管程序集加载（原生 PE 抛 BadImageFormatException
            // 导致 mod 禁用）；.sim 绕过该扫描，且 Windows LoadLibrary 支持任意扩展名。
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

            // 安装原生 detour：接管原版 SimDLL.dll 的 16 个导出。
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

        private static class NativeMethods
        {
            [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
            public static extern IntPtr LoadLibrary(string lpFileName);
        }
    }
}
