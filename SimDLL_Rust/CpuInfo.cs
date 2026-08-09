using System;
using System.Runtime.InteropServices;

namespace SimDLL_Rust
{
    /// <summary>
    /// CPU 超线程检测（P/Invoke GetLogicalProcessorInformationEx +
    /// LTP_HYPER_THREADING_PRESENT 标志）。
    /// 供 Options 屏判断"启用超线程优化"勾选项是否可勾选（无超线程 → 灰显禁用）。
    /// 检测失败/异常 → 保守返回 false（禁用方向 1）。
    /// 注：旧 API GetLogicalProcessorInformation 在本机（Win11/部分环境）不返回
    /// ProcessorCore 条目（全为 Group），导致检测恒 false——故改用 Ex 版。
    /// </summary>
    public static class CpuInfo
    {
        private const uint RELATION_PROCESSOR_CORE = 0;
        private const uint LTP_HYPER_THREADING_PRESENT = 0x1;

        [StructLayout(LayoutKind.Sequential)]
        private struct SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX
        {
            public uint Relationship;
            public uint Size;
            // PROCESSOR_RELATIONSHIP 起始（offset 8）：Flags 为第 1 字节。
            public byte ProcessorFlags;
            public byte EfficiencyClass;
            [MarshalAs(UnmanagedType.ByValArray, SizeConst = 20)]
            public byte[] Reserved;
            public ushort GroupCount;
            // GROUP_AFFINITY[1]（8 字节）
            public ushort GroupMask;
            public ushort GroupReserved1;
            public ushort GroupReserved2;
            public ushort GroupReserved3;
        }

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetLogicalProcessorInformationEx(
            uint relationship, IntPtr buffer, ref uint returnedLength);

        /// <summary>CPU 是否启用了超线程（任一物理核的逻辑处理器数 &gt; 1）。</summary>
        public static bool HasHyperThreading()
        {
            try
            {
                uint len = 0;
                if (!GetLogicalProcessorInformationEx(RELATION_PROCESSOR_CORE, IntPtr.Zero, ref len)
                    || len == 0)
                {
                    return false;
                }
                IntPtr buffer = Marshal.AllocHGlobal((int)len);
                try
                {
                    if (!GetLogicalProcessorInformationEx(
                        RELATION_PROCESSOR_CORE, buffer, ref len))
                    {
                        return false;
                    }
                    int size = Marshal.SizeOf<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>();
                    int offset = 0;
                    while (offset + size <= (int)len)
                    {
                        var info = Marshal.PtrToStructure<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>(
                            buffer + offset);
                        if (info.Relationship == RELATION_PROCESSOR_CORE
                            && (info.ProcessorFlags & LTP_HYPER_THREADING_PRESENT) != 0)
                        {
                            return true;
                        }
                        if (info.Size <= 0)
                        {
                            break;
                        }
                        offset += (int)info.Size;
                    }
                    return false;
                }
                finally
                {
                    Marshal.FreeHGlobal(buffer);
                }
            }
            catch
            {
                return false;
            }
        }
    }
}
