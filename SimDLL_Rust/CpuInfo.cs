using System;
using System.Runtime.InteropServices;

namespace SimDLL_Rust
{
    /// <summary>
    /// CPU 超线程检测（P/Invoke GetLogicalProcessorInformation）。
    /// 供 Options 屏判断"启用超线程优化"勾选项是否可勾选（无超线程 → 灰显禁用）。
    /// 检测失败/异常 → 保守返回 false（禁用方向 1）。
    /// </summary>
    public static class CpuInfo
    {
        private const uint RelationProcessorCore = 0;

        [StructLayout(LayoutKind.Sequential)]
        private struct SYSTEM_LOGICAL_PROCESSOR_INFORMATION
        {
            public uint Relationship;
            public UIntPtr ProcessorMask;
            // union（8 字节）：这里只关心 Relationship/ProcessorMask，取前 8 字节联合。
            public uint UnionA;
            public uint UnionB;
        }

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetLogicalProcessorInformation(IntPtr buffer, ref uint returnedLength);

        /// <summary>CPU 是否启用了超线程（任一物理核的逻辑处理器数 &gt; 1）。</summary>
        public static bool HasHyperThreading()
        {
            try
            {
                uint len = 0;
                if (!GetLogicalProcessorInformation(IntPtr.Zero, ref len) || len == 0)
                {
                    return false;
                }
                IntPtr buffer = Marshal.AllocHGlobal((int)len);
                try
                {
                    if (!GetLogicalProcessorInformation(buffer, ref len))
                    {
                        return false;
                    }
                    int size = Marshal.SizeOf<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
                    int count = (int)len / size;
                    for (int i = 0; i < count; i++)
                    {
                        var info = Marshal.PtrToStructure<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>(
                            buffer + i * size);
                        if (info.Relationship == RelationProcessorCore
                            && PopCount(info.ProcessorMask.ToUInt64()) > 1)
                        {
                            return true;
                        }
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

        private static int PopCount(ulong v)
        {
            int c = 0;
            while (v != 0)
            {
                v &= v - 1;
                c++;
            }
            return c;
        }
    }
}
