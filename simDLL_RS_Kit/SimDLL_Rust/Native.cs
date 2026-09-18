using System;
using System.Runtime.InteropServices;

namespace SimDLL_Rust
{
    /// <summary>
    /// Rust simDLL（Windows: SimDLL_RS.sim / Linux: SimDLL_RS.so）的原生入口绑定。
    ///
    /// 2026-09-09 跨平台改造：配置函数不再用静态 DllImport（其名字解析在
    /// Windows 依赖 LoadLibrary 按 base-name 命中已加载模块、Linux Mono 下
    /// 对非 lib*.so 名不可靠），改为**加载成功后按符号动态绑定**
    /// （Windows: GetProcAddress / Linux: dlsym）+ delegate 调用。
    /// Sim API（16 个）仅为 ABI 参照保留 DllImport 声明——游戏侧经原生 detour
    /// 直调 Rust 实现，C# 无调用者；DllImport 惰性解析不触发。
    /// 签名与游戏 Assembly-CSharp 的 Sim / ConduitTemperatureManager extern
    /// 逐一对应（ABI 一致，extern "C" cdecl）。
    /// </summary>
    internal static unsafe class Native
    {
        // Rust 导出均为 extern "C"（cdecl）。
        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        internal delegate int IntFn();

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        internal delegate int IntFromIntFn(int value);

        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        internal delegate int IntFromBoolFn(bool enabled);

        private static IntFn _loaderInstall;
        private static IntFromBoolFn _setLogEnabled;
        private static IntFromIntFn _setSchedulerMode;
        private static IntFromIntFn _setReservedCores;

        /// 原生库是否已加载且符号解析完成。失败（如不支持的架构）时为 false，
        /// 所有入口短路返回失败值——调用方据此回落原版 sim。
        internal static bool IsAvailable { get; private set; }

        /// 在已加载的原生模块上解析配置函数符号（Loader 加载成功后调用一次）。
        internal static void Resolve(IntPtr nativeModule)
        {
            try
            {
                _loaderInstall = Marshal.GetDelegateForFunctionPointer<IntFn>(
                    GetAddress(nativeModule, "RS_LoaderInstall"));
                _setLogEnabled = Marshal.GetDelegateForFunctionPointer<IntFromBoolFn>(
                    GetAddress(nativeModule, "RS_SetLogEnabled"));
                _setSchedulerMode = Marshal.GetDelegateForFunctionPointer<IntFromIntFn>(
                    GetAddress(nativeModule, "RS_SetSchedulerMode"));
                _setReservedCores = Marshal.GetDelegateForFunctionPointer<IntFromIntFn>(
                    GetAddress(nativeModule, "RS_SetReservedCores"));
                IsAvailable = true;
            }
            catch (Exception e)
            {
                UnityEngine.Debug.LogError("[SimDLL_Rust] 原生符号解析失败: " + e.Message);
                IsAvailable = false;
            }
        }

        internal static bool IsWindowsPlatform()
        {
            switch (Environment.OSVersion.Platform)
            {
                case PlatformID.Win32NT:
                case PlatformID.Win32Windows:
                case PlatformID.Win32S:
                case PlatformID.WinCE:
                    return true;
                default:
                    return false;
            }
        }

        private static IntPtr GetAddress(IntPtr module, string name)
        {
            if (IsWindowsPlatform())
            {
                return GetProcAddress(module, name);
            }
            return dlsym(module, name);
        }

        [DllImport("kernel32", CharSet = CharSet.Ansi, SetLastError = true)]
        private static extern IntPtr GetProcAddress(IntPtr module, string name);

        // Linux：libc.so.6 必然已被主进程加载（ONI Linux 版基于 glibc 发行版），
        // dlopen 短名命中已加载副本。
        [DllImport("libc.so.6", SetLastError = true)]
        private static extern IntPtr dlsym(IntPtr handle, string symbol);

        /// 安装原生 detour（接管原版 SimDLL 的 16 个导出）。返回安装数。
        internal static int RS_LoaderInstall() => _loaderInstall();

        /// 日志开关（发布版无等级，纯开/关）。
        internal static int RS_SetLogEnabled(bool enabled) => _setLogEnabled(enabled);

        /// 设置调度模式：0=Auto，1=D2（rayon 装箱），2=D1（1 星 1 线程 + 超线程）。
        /// 返回实际生效模式（D1 无超线程时回退 D2 返回 1）。
        internal static int RS_SetSchedulerMode(int mode) => _setSchedulerMode(mode);

        /// 设置预留物理线程数（1~4）：留给游戏主线程/系统，D1/D2 并行路径通用。
        /// 返回实际生效值；非法输入返回 -1。
        internal static int RS_SetReservedCores(int cores) => _setReservedCores(cores);

        // ===== Sim API（9）—— ABI 参照声明，C# 无调用者（游戏经 detour 直调）=====
        [DllImport("SimDLL_RS.sim")]
        internal static extern void SIM_Initialize(Sim.GAME_MessageHandler callback);

        [DllImport("SimDLL_RS.sim")]
        internal static extern void SIM_Shutdown();

        [DllImport("SimDLL_RS.sim")]
        internal static extern IntPtr SIM_HandleMessage(int sim_msg_id, int msg_length, byte* msg);

        [DllImport("SimDLL_RS.sim")]
        internal static extern IntPtr SIM_HandleMessages(int sim_msg_id, int msg_length, int msg_count, byte* msg);

        [DllImport("SimDLL_RS.sim")]
        internal static extern byte* SIM_BeginSave(int* size, int x, int y);

        [DllImport("SimDLL_RS.sim")]
        internal static extern void SIM_EndSave();

        [DllImport("SimDLL_RS.sim")]
        internal static extern void SIM_DebugCrash();

        [DllImport("SimDLL_RS.sim")]
        internal static extern char* SYSINFO_Acquire();

        [DllImport("SimDLL_RS.sim")]
        internal static extern void SYSINFO_Release();

        // ===== ConduitTemperatureManager API（7）=====
        [DllImport("SimDLL_RS.sim")]
        internal static extern void ConduitTemperatureManager_Initialize();

        [DllImport("SimDLL_RS.sim")]
        internal static extern void ConduitTemperatureManager_Shutdown();

        [DllImport("SimDLL_RS.sim")]
        internal static extern int ConduitTemperatureManager_Add(
            float contents_temperature,
            float contents_mass,
            int contents_element_hash,
            int conduit_structure_temperature_handle,
            float conduit_heat_capacity,
            float conduit_thermal_conductivity,
            bool conduit_insulated);

        [DllImport("SimDLL_RS.sim")]
        internal static extern int ConduitTemperatureManager_Set(
            int handle,
            float contents_temperature,
            float contents_mass,
            int contents_element_hash);

        [DllImport("SimDLL_RS.sim")]
        internal static extern void ConduitTemperatureManager_Remove(int handle);

        [DllImport("SimDLL_RS.sim")]
        internal static extern IntPtr ConduitTemperatureManager_Update(float dt, IntPtr building_conductivity_data);

        [DllImport("SimDLL_RS.sim")]
        internal static extern void ConduitTemperatureManager_Clear();
    }
}
