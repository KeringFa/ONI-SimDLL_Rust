using System;
using System.Runtime.InteropServices;
using UnityEngine;

namespace SimDLL_Rust
{
    /// <summary>
    /// CPU 超线程检测（P/Invoke GetLogicalProcessorInformationEx +
    /// LTP_HYPER_THREADING_PRESENT 标志）。
    /// 供 Options 屏判断"启用超线程优化"勾选项是否可勾选（无超线程 → 灰显禁用）。
    /// 检测失败/异常 → 保守返回 false（禁用方向 1）。
    /// 注：旧 API GetLogicalProcessorInformation 在本机（Win11/部分环境）不返回
    /// ProcessorCore 条目（全为 Group），导致检测恒 false——故改用 Ex 版；
    /// 字段读取用手写偏移（不用 Marshal 结构布局，规避 Mono/.NET 布局差异）。
    /// </summary>
    public static class CpuInfo
    {
        private const uint RELATION_PROCESSOR_CORE = 0;
        private const uint LTP_HYPER_THREADING_PRESENT = 0x1;

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetLogicalProcessorInformationEx(
            uint relationship, IntPtr buffer, ref uint returnedLength);

        /// <summary>CPU 是否启用了超线程（任一物理核的逻辑处理器数 &gt; 1）。</summary>
        public static bool HasHyperThreading()
        {
            try
            {
                uint len = 0;
                // 第一次调用：查询所需缓冲区大小。预期返回 false
                // （ERROR_INSUFFICIENT_BUFFER），len 被设置为所需字节数——
                // 此时不能把 false 当失败。
                GetLogicalProcessorInformationEx(
                    RELATION_PROCESSOR_CORE, IntPtr.Zero, ref len);
                if (len == 0)
                {
                    Debug.Log("[SimDLL_Rust] CpuInfo: query length failed (len=0)");
                    return false;
                }
                IntPtr buffer = Marshal.AllocHGlobal((int)len);
                try
                {
                    if (!GetLogicalProcessorInformationEx(
                        RELATION_PROCESSOR_CORE, buffer, ref len))
                    {
                        Debug.Log("[SimDLL_Rust] CpuInfo: second call failed");
                        return false;
                    }
                    int offset = 0;
                    int cores = 0;
                    int htCores = 0;
                    while (offset + 8 <= (int)len)
                    {
                        // Windows ABI：EX 条目头部 Relationship@0、Size@4；
                        // RelationProcessorCore 的 PROCESSOR_RELATIONSHIP.Flags 在 offset+8。
                        uint relationship = (uint)Marshal.ReadInt32(buffer, offset);
                        uint size = (uint)Marshal.ReadInt32(buffer, offset + 4);
                        if (relationship == RELATION_PROCESSOR_CORE)
                        {
                            cores++;
                            byte flags = Marshal.ReadByte(buffer, offset + 8);
                            if ((flags & LTP_HYPER_THREADING_PRESENT) != 0)
                            {
                                htCores++;
                            }
                        }
                        if (size == 0)
                        {
                            break;
                        }
                        offset += (int)size;
                    }
                    // 完整扫描后输出总物理核数（避免"首个命中核序号"被误读为总核数，
                    // 2026-08-21 玩家反馈 7800X3D 显示 cores=1 的误导修复）。
                    if (htCores > 0)
                    {
                        Debug.Log("[SimDLL_Rust] CpuInfo: HT present, cores=" + cores
                            + " htCores=" + htCores);
                        return true;
                    }
                    Debug.Log("[SimDLL_Rust] CpuInfo: no HT flags (cores=" + cores
                        + ", htCores=" + htCores + ", len=" + len + ")");
                    return false;
                }
                finally
                {
                    Marshal.FreeHGlobal(buffer);
                }
            }
            catch (Exception e)
            {
                Debug.Log("[SimDLL_Rust] CpuInfo: exception " + e.GetType().Name + ": " + e.Message);
                return false;
            }
        }
    }
}
