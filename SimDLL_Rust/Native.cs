using System;
using System.Runtime.InteropServices;

namespace SimDLL_Rust
{
    /// <summary>
    /// Rust simDLL（SimDLL_RS.sim）的原生入口声明。签名与游戏 Assembly-CSharp
    /// 的 Sim / ConduitTemperatureManager extern 逐一对应（ABI 一致）。
    /// </summary>
    internal static unsafe class Native
    {
        /// 安装原生 detour（接管原版 SimDLL.dll 的 16 个导出）。返回安装数。
        [DllImport("SimDLL_RS.sim")]
        internal static extern int RS_LoaderInstall();

        /// 动态设置 Rust 日志级别（UTF-8 NUL 结尾：off/error/warn/info/debug/trace）。
        [DllImport("SimDLL_RS.sim")]
        internal static extern int RS_SetLogLevel(byte[] level);

        /// 设置调度模式：0=Auto，1=D2（rayon 装箱），2=D1（1 星 1 线程 + 超线程）。
        /// 返回实际生效模式（D1 无超线程时回退 D2 返回 1）。
        [DllImport("SimDLL_RS.sim")]
        internal static extern int RS_SetSchedulerMode(int mode);

        // ===== Sim API（9）=====
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
