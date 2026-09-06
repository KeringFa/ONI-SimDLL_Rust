using HarmonyLib;
using KMod;
using System;
using System.Collections.Generic;
using System.Diagnostics;
using UnityEngine;

namespace SimDLL_PerfProbe
{
    /// <summary>
    /// 主线程性能热点探针 v4（2026-09-06，RENDERGAP 黑盒拆解）：
    /// - union 归因：深度计数器使 RENDERGAP = FRAME − 最外层探针方法合计，
    ///   修正 v3 嵌套链（Game.Update⊃SimEveryTick⊃…）重复计入导致的系统性低估。
    /// - 双账本（detail/union）乒乓交换，稳态零分配（v3 每帧 new Dictionary 污染 GC）。
    /// - GC 归因：帧间隔与 Dump 窗口的 GC.CollectionCount delta，尖刺带 gc 标记。
    /// - 补测裸奔方法：LogicCircuitManager/SolidConduitFlow.RenderEveryTick、
    ///   AsyncPathProber.Manager.TickFrame（均在 Game.Update 调用链内）。
    /// - 窗口行带环境口径：fpsCap/vsync/gc delta。
    /// v3 存量能力：相机移动分桶（M./S.）、32 格分块跨界检测、纯测量不改行为。
    /// </summary>
    public class PerfProbeMod : UserMod2
    {
        public override void OnLoad(Harmony harmony)
        {
            base.OnLoad(harmony);
            PatchRuntimeRenderMethods(harmony);
            PatchUgui(harmony);
        }

        /// v4.1：GAP 拆解补盲——uGUI 画布重建（含 TMP 文本网格），运行时 patch
        /// （UnityEngine.UI 是独立程序集，CanvasUpdateRegistry 为 internal，用 TypeByName）。
        private static void PatchUgui(Harmony harmony)
        {
            var registry = AccessTools.TypeByName("UnityEngine.UI.CanvasUpdateRegistry");
            if (registry == null)
            {
                UnityEngine.Debug.Log("[PerfProbe] CanvasUpdateRegistry not found, skip uGUI probe");
                return;
            }
            var perform = AccessTools.Method(registry, "PerformUpdate");
            if (perform != null)
            {
                harmony.Patch(
                    perform,
                    prefix: new HarmonyMethod(typeof(P_CanvasPerformUpdate), nameof(P_CanvasPerformUpdate.Prefix)),
                    postfix: new HarmonyMethod(typeof(P_CanvasPerformUpdate), nameof(P_CanvasPerformUpdate.Postfix)));
            }
            else
            {
                UnityEngine.Debug.Log("[PerfProbe] CanvasUpdateRegistry.PerformUpdate not found, skip");
            }
        }

        /// 运行时 patch 二进制 DLL 中的渲染方法（KBatchedAnimUpdater 不在反编译
        /// 源码里，无法用编译期 [HarmonyPatch] 属性，改用 AccessTools 动态定位）。
        private static void PatchRuntimeRenderMethods(Harmony harmony)
        {
            var updater = AccessTools.TypeByName("KBatchedAnimUpdater");
            if (updater == null)
            {
                UnityEngine.Debug.Log("[PerfProbe] KBatchedAnimUpdater type not found, skip");
                return;
            }
            var late = AccessTools.Method(updater, "LateUpdate");
            if (late != null)
            {
                harmony.Patch(
                    late,
                    prefix: new HarmonyMethod(typeof(P_KBatchedAnimUpdaterLateUpdate), nameof(P_KBatchedAnimUpdaterLateUpdate.Prefix)),
                    postfix: new HarmonyMethod(typeof(P_KBatchedAnimUpdaterLateUpdate), nameof(P_KBatchedAnimUpdaterLateUpdate.Postfix)));
            }
            var getVis = AccessTools.Method(updater, "GetVisibleArea");
            if (getVis != null)
            {
                harmony.Patch(
                    getVis,
                    prefix: new HarmonyMethod(typeof(P_KBatchedAnimUpdaterGetVisibleArea), nameof(P_KBatchedAnimUpdaterGetVisibleArea.Prefix)),
                    postfix: new HarmonyMethod(typeof(P_KBatchedAnimUpdaterGetVisibleArea), nameof(P_KBatchedAnimUpdaterGetVisibleArea.Postfix)));
            }
        }
    }

    public static class Probe
    {
        private const long WINDOW_MS = 5000;
        private const float MOVE_THRESHOLD = 0.02f; // 相机每帧位移 > 0.02 格视为移动帧
        private const long SPIKE_THRESHOLD_US = 16_000; // >16.7ms 帧预算视为尖刺
        private const long FRAME_SPIKE_THRESHOLD_US = 10_000; // 165Hz 帧预算 ~6ms；>10ms = 掉帧尖刺
        private const long FRAME_BREAKDOWN_THRESHOLD_US = 100_000; // >100ms 大尖刺 → 输出帧内方法分解
        private const int CHUNK_CELLS = 32; // KAnimBatchManager.CHUNK_SIZE

        private static readonly Dictionary<string, Sample> samples = new();
        private static readonly List<string> spikes = new();
        // detail 账本：所有探针方法（含嵌套）——用于 >100ms 大尖刺的帧内分解明细
        private static Dictionary<string, long> curFrameMethods = new();
        private static Dictionary<string, long> prevFrameMethods = new();
        // union 账本：仅最外层探针方法——RENDERGAP = FRAME − union（v4 修正嵌套重复计入）
        private static Dictionary<string, long> curUnionMethods = new();
        private static Dictionary<string, long> prevUnionMethods = new();
        private static readonly Stopwatch window = Stopwatch.StartNew();

        // 探针方法嵌套深度：Begin 加一、Record 减一；Record 时 ==1 表示最外层方法
        private static int depth;
        // 本帧间隔内 GC collection delta（P_GameUpdate.Prefix 经 FrameBoundary 写入）
        internal static long frameGc0;
        internal static long frameGc1;
        internal static long frameGc2;
        private static long lastGc0, lastGc1, lastGc2; // 上一帧边界快照
        private static long winGc0, winGc1, winGc2;   // 上一 Dump 窗口快照
        // FastTrack 在场检测：-1 未知 / 0 未加载 / 1 已加载（决定测量口径解释）
        private static int fastTrackLoaded = -1;

        internal static Vector3 lastCameraPos;
        internal static bool IsCameraMoving;
        internal static int CamChunkX;
        internal static int CamChunkY;
        internal static bool ChunkCrossed;
        internal static long LastFrameTs;

        private struct Sample
        {
            public long totalUs;
            public int count;
            public long maxUs;
        }

        internal static long Begin()
        {
            depth++;
            return Stopwatch.GetTimestamp();
        }

        /// 帧边界（P_GameUpdate.Prefix 在 Probe.Begin 之前调用）：
        /// 深度泄漏防护（异常帧 Postfix 缺失不会永久污染归因）+ 帧间隔 GC delta。
        internal static void FrameBoundary()
        {
            depth = 0;
            long g0 = GC.CollectionCount(0);
            long g1 = GC.CollectionCount(1);
            long g2 = GC.CollectionCount(2);
            frameGc0 = g0 - lastGc0;
            frameGc1 = g1 - lastGc1;
            frameGc2 = g2 - lastGc2;
            lastGc0 = g0;
            lastGc1 = g1;
            lastGc2 = g2;
        }

        internal static long EndUs(long startTs)
        {
            long ticks = Stopwatch.GetTimestamp() - startTs;
            return ticks * 1_000_000 / Stopwatch.Frequency;
        }

        internal static void TrackCamera(Vector3 pos)
        {
            IsCameraMoving = (pos - lastCameraPos).sqrMagnitude > MOVE_THRESHOLD * MOVE_THRESHOLD;
            lastCameraPos = pos;
        }

        /// 检测可见区左/下边缘是否跨过 32 格分块边界（KAnim 批量重建触发帧）。
        internal static void TrackChunk(Vector3 camPos, float halfH)
        {
            float halfW = halfH * (Camera.main != null ? Camera.main.aspect : 1f);
            int chunkX = ((int)System.Math.Floor(camPos.x - halfW)) / CHUNK_CELLS;
            int chunkY = ((int)System.Math.Floor(camPos.y - halfH)) / CHUNK_CELLS;
            ChunkCrossed = chunkX != CamChunkX || chunkY != CamChunkY;
            CamChunkX = chunkX;
            CamChunkY = chunkY;
        }

        internal static void Record(string key, long us)
        {
            bool isOutermost = depth == 1;
            if (depth > 0) depth--; // 与 Begin 配对；先判定最外层再递减
            string k = (IsCameraMoving ? "M." : "S.") + key;
            if (!samples.TryGetValue(k, out var s))
            {
                s = new Sample();
            }
            s.totalUs += us;
            s.count++;
            if (us > s.maxUs) s.maxUs = us;
            samples[k] = s;

            // detail 账本：无条件累计（大尖刺分解用，含嵌套）
            if (!curFrameMethods.TryGetValue(k, out var prevUs))
            {
                prevUs = 0;
            }
            curFrameMethods[k] = prevUs + us;

            // union 账本：仅最外层方法计入帧归因（嵌套者耗时已含在外层里）
            if (isOutermost)
            {
                if (!curUnionMethods.TryGetValue(k, out var prevUnionUs))
                {
                    prevUnionUs = 0;
                }
                curUnionMethods[k] = prevUnionUs + us;
            }

            if (us > SPIKE_THRESHOLD_US)
            {
                spikes.Add(
                    k + " " + (us / 1000) + "ms move=" + IsCameraMoving
                        + " chunkCross=" + ChunkCrossed
                        + " chunk=(" + CamChunkX + "," + CamChunkY + ")"
                );
            }

            if (window.ElapsedMilliseconds >= WINDOW_MS)
            {
                Dump();
            }
        }


        /// 仅计数（无耗时）：CAM.Frame=总帧数、CAM.Move=移动帧数。
        internal static void Count(string key)
        {
            if (!samples.TryGetValue(key, out var s))
            {
                s = new Sample();
            }
            s.count++;
            samples[key] = s;

            if (window.ElapsedMilliseconds >= WINDOW_MS)
            {
                Dump();
            }
        }

        /// 记录整帧耗时（相邻 Game.Update 前缀的时间差 = 完整帧时间，含渲染/GPU 提交）。
        /// >10ms 视为掉帧尖刺（165Hz 预算 ~6ms），与 move/chunkCross 一起进尖刺日志。
        internal static void RecordFrame(long us)
        {
            string k = (IsCameraMoving ? "M." : "S.") + "FRAME";
            if (!samples.TryGetValue(k, out var s))
            {
                s = new Sample();
            }
            s.totalUs += us;
            s.count++;
            if (us > s.maxUs) s.maxUs = us;
            samples[k] = s;

            // 渲染间隙统计（v4 union 口径）：FRAME − 上一帧最外层探针方法合计
            // = 渲染/GPU/未探针时间。旧版用含嵌套的 Σ 系统性低估 RENDERGAP。
            long gapMethodsUs = 0;
            foreach (var kv in prevUnionMethods)
            {
                gapMethodsUs += kv.Value;
            }
            long gap = us - gapMethodsUs;
            if (gap < 0) gap = 0;
            string gapKey = (IsCameraMoving ? "M." : "S.") + "RENDERGAP";
            if (!samples.TryGetValue(gapKey, out var g))
            {
                g = new Sample();
            }
            g.totalUs += gap;
            g.count++;
            if (gap > g.maxUs) g.maxUs = gap;
            samples[gapKey] = g;

            if (us > FRAME_SPIKE_THRESHOLD_US)
            {
                string gcTag = (frameGc0 == 0 && frameGc1 == 0 && frameGc2 == 0)
                    ? " gc=0"
                    : " gc0+" + frameGc0 + " gc1+" + frameGc1 + " gc2+" + frameGc2;
                spikes.Add(
                    k + " " + (us / 1000) + "ms move=" + IsCameraMoving
                        + " chunkCross=" + ChunkCrossed
                        + " chunk=(" + CamChunkX + "," + CamChunkY + ")"
                        + gcTag
                        + " t=" + window.ElapsedMilliseconds + "ms"
                );
            }

            if (us > FRAME_BREAKDOWN_THRESHOLD_US)
            {
                // 大尖刺归因：上一帧各被探针方法耗时 + 渲染/未探针间隙
                long methodsUs = 0;
                var parts = new System.Text.StringBuilder();
                foreach (var kv in prevFrameMethods)
                {
                    methodsUs += kv.Value;
                    parts.Append(kv.Key).Append('=').Append(kv.Value / 1000).Append("ms ");
                }
                spikes.Add(
                    k + " " + (us / 1000) + "ms methods=" + (methodsUs / 1000) + "ms"
                        + " renderGap=" + ((us - methodsUs) / 1000) + "ms ["
                        + parts + "] t=" + window.ElapsedMilliseconds + "ms"
                );
            }

            // 账本乒乓交换（稳态零分配）：detail 与 union 各一对
            var tmp = prevFrameMethods;
            prevFrameMethods = curFrameMethods;
            curFrameMethods = tmp;
            curFrameMethods.Clear();

            var tmpUnion = prevUnionMethods;
            prevUnionMethods = curUnionMethods;
            curUnionMethods = tmpUnion;
            curUnionMethods.Clear();

            if (window.ElapsedMilliseconds >= WINDOW_MS)
            {
                Dump();
            }
        }

        private static void Dump()
        {
            long winMs = window.ElapsedMilliseconds;
            window.Restart();
            winGc0 = GC.CollectionCount(0);
            winGc1 = GC.CollectionCount(1);
            winGc2 = GC.CollectionCount(2);
            long totalUs = 0;
            foreach (var kv in samples) totalUs += kv.Value.totalUs;

            var sb = new System.Text.StringBuilder();
            if (fastTrackLoaded < 0)
            {
                fastTrackLoaded = AccessTools.TypeByName("PeterHan.FastTrack.FastTrackMod") != null
                    ? 1 : 0;
            }
            sb.Append("[PerfProbe v4] window=").Append(winMs).Append("ms")
              .Append(" ft=").Append(fastTrackLoaded)
              .Append(" fpsCap=").Append(Application.targetFrameRate)
              .Append(" vsync=").Append(QualitySettings.vSyncCount)
              .Append(" gc0+").Append(GC.CollectionCount(0) - winGc0)
              .Append(" gc1+").Append(GC.CollectionCount(1) - winGc1)
              .Append(" gc2+").Append(GC.CollectionCount(2) - winGc2)
              .Append(" envThreads=").Append(Environment.ProcessorCount)
              .Append(" total=").Append(totalUs / 1000).Append("ms | ");
            foreach (var kv in samples)
            {
                var s = kv.Value;
                sb.Append(kv.Key)
                  .Append(':').Append(s.totalUs / 1000).Append("ms (")
                  .Append(s.count).Append("x, avg=")
                  .Append(s.count > 0 ? s.totalUs / s.count / 1000 : 0).Append("ms, max=")
                  .Append(s.maxUs / 1000).Append("ms); ");
            }
            Debug.Log(sb.ToString());
            if (spikes.Count > 0)
            {
                var spikeSb = new System.Text.StringBuilder();
                spikeSb.Append("[PerfProbe] SPIKES[").Append(spikes.Count).Append("]: ");
                foreach (var s in spikes)
                {
                    spikeSb.Append(s).Append("; ");
                }
                Debug.Log(spikeSb.ToString());
                try
                {
                    string dir = System.IO.Path.GetDirectoryName(
                        System.Reflection.Assembly.GetExecutingAssembly().Location) ?? ".";
                    System.IO.File.AppendAllText(
                        System.IO.Path.Combine(dir, "perf.log"),
                        System.DateTime.Now.ToString("HH:mm:ss") + " " + spikeSb.ToString() + System.Environment.NewLine);
                }
                catch
                {
                }
            }
            spikes.Clear();
            try
            {
                string dir = System.IO.Path.GetDirectoryName(
                    System.Reflection.Assembly.GetExecutingAssembly().Location) ?? ".";
                System.IO.File.AppendAllText(
                    System.IO.Path.Combine(dir, "perf.log"),
                    System.DateTime.Now.ToString("HH:mm:ss") + " " + sb.ToString() + System.Environment.NewLine);
            }
            catch
            {
                // 写文件失败不影响 Debug.Log 输出
            }
            samples.Clear();
        }
    }

    // ===== 总帧成本 + 相机移动检测 =====
    [HarmonyPatch(typeof(Game), "Update")]
    internal static class P_GameUpdate
    {
        internal static void Prefix(out long __state)
        {
            // 帧边界：深度复位 + GC delta（必须在 Probe.Begin 之前，
            // 否则 Game.Update 自身会被判为嵌套而失去 union 归因）
            Probe.FrameBoundary();
            // 整帧耗时 = 本帧与上一帧 Game.Update 的时间差（含渲染/GPU 提交）
            long now = Probe.Begin();
            if (Probe.LastFrameTs != 0)
            {
                Probe.RecordFrame(Probe.EndUs(Probe.LastFrameTs));
            }
            Probe.LastFrameTs = now;
            var cam = CameraController.Instance;
            if (cam != null)
            {
                var pos = cam.transform.position;
                Probe.TrackCamera(pos);
                Probe.TrackChunk(pos, cam.OrthographicSize);
            }
            else
            {
                Probe.TrackCamera(Vector3.zero);
            }
            Probe.Count("CAM.Frame");
            if (Probe.IsCameraMoving) Probe.Count("CAM.Move");
            __state = now;
        }
        internal static void Postfix(long __state) { Probe.Record("Game.Update", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(Game), "LateUpdate")]
    internal static class P_GameLateUpdate
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("Game.LateUpdate", Probe.EndUs(__state)); }
    }

    // ===== Game：sim 同步链路 =====
    [HarmonyPatch(typeof(Game), "SimEveryTick")]
    internal static class P_SimEveryTick
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("Game.SimEveryTick", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(Game), "UnsafeSim200ms")]
    internal static class P_UnsafeSim200ms
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("Game.UnsafeSim200ms", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(Game), "StepTheSim")]
    internal static class P_StepTheSim
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("Game.StepTheSim", Probe.EndUs(__state)); }
    }

    // ===== 实体状态机 =====
    [HarmonyPatch(typeof(StateMachineUpdater), "AdvanceOneSimSubTick")]
    internal static class P_AdvanceOneSimSubTick
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("SM.AdvanceOneSimSubTick", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(StateMachineUpdater), "RenderEveryTick")]
    internal static class P_SM_RenderEveryTick
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("SM.RenderEveryTick", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(StateMachineUpdater), "Render")]
    internal static class P_SM_Render
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("SM.Render", Probe.EndUs(__state)); }
    }

    // ===== 渲染管线（相机移动尖峰嫌疑区）=====
    [HarmonyPatch(typeof(World), "LateUpdate")]
    internal static class P_WorldLateUpdate
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("World.LateUpdate", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(
        typeof(KAnimBatchManager),
        "UpdateActiveArea",
        new Type[] { typeof(Vector2I), typeof(Vector2I) })]
    internal static class P_KAnimUpdateActiveArea
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("KAnim.UpdateActiveArea", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(KAnimBatchManager), "UpdateDirty", new Type[] { typeof(int) })]
    internal static class P_KAnimUpdateDirty
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("KAnim.UpdateDirty", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(KAnimBatchManager), "Render")]
    internal static class P_KAnimRender
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("KAnim.Render", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(
        typeof(GroundRenderer),
        "Render",
        new Type[] { typeof(Vector2I), typeof(Vector2I), typeof(bool) })]
    internal static class P_GroundRendererRender
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("GroundRenderer.Render", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(PropertyTextures), "LateUpdate")]
    internal static class P_PropertyTexturesLateUpdate
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("PropertyTextures.LateUpdate", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(WaterCubes), "LateUpdate")]
    internal static class P_WaterCubesLateUpdate
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("WaterCubes.LateUpdate", Probe.EndUs(__state)); }
    }

    // ===== 渲染提交（相机移动时 +3~4ms 的盲区嫌疑）=====
    [HarmonyPatch(typeof(Camera), "Render", new Type[0])]
    internal static class P_CameraRender
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("Camera.Render", Probe.EndUs(__state)); }
    }

    /// KBatchedAnimUpdater.LateUpdate（二进制 DLL，运行时 patch）——实体批处理
    /// 逐帧更新；相机移动时实体屏幕位置变化，嫌疑最大。
    internal static class P_KBatchedAnimUpdaterLateUpdate
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("KBatchedAnimUpdater.LateUpdate", Probe.EndUs(__state)); }
    }

    /// KBatchedAnimUpdater.GetVisibleArea（二进制 DLL，运行时 patch）。
    internal static class P_KBatchedAnimUpdaterGetVisibleArea
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("KBatchedAnimUpdater.GetVisibleArea", Probe.EndUs(__state)); }
    }

    /// v4.1：uGUI 画布重建总成本（CanvasUpdateRegistry.PerformUpdate，运行时 patch）。
    /// 该方法在 player loop 中位于 Game.Update 链之外 → union 归因，直接从 RENDERGAP 里分账。
    internal static class P_CanvasPerformUpdate
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("UI.CanvasPerformUpdate", Probe.EndUs(__state)); }
    }

    // ===== 管道/电路网络（每帧）=====
    [HarmonyPatch(typeof(UtilityNetworkManager<FlowUtilityNetwork, Vent>), "Update")]
    internal static class P_GasConduitUpdate
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("GasConduit.Update", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(UtilityNetworkManager<FlowUtilityNetwork, SolidConduit>), "Update")]
    internal static class P_SolidConduitUpdate
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("SolidConduit.Update", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(UtilityNetworkManager<ElectricalUtilityNetwork, Wire>), "Update")]
    internal static class P_ElectricalUpdate
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("Electrical.Update", Probe.EndUs(__state)); }
    }

    // ===== v4 补测：Game.Update 调用链内的裸奔方法（签名已对反编译源核实） =====
    [HarmonyPatch(typeof(LogicCircuitManager), "RenderEveryTick", new Type[] { typeof(float) })]
    internal static class P_LogicCircuitRenderEveryTick
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("LogicCircuit.RenderEveryTick", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(SolidConduitFlow), "RenderEveryTick", new Type[] { typeof(float) })]
    internal static class P_SolidConduitFlowRenderEveryTick
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("SolidConduitFlow.RenderEveryTick", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(AsyncPathProber.Manager), "TickFrame")]
    internal static class P_AsyncPathProberTickFrame
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("AsyncPathProber.TickFrame", Probe.EndUs(__state)); }
    }

    // ===== 寻路 / 变更监控 / 纹理 =====
    [HarmonyPatch(typeof(Pathfinding), "UpdateNavGrids")]
    internal static class P_UpdateNavGrids
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("Pathfinding.UpdateNavGrids", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(CellChangeMonitor), "RenderEveryTick")]
    internal static class P_CellChangeMonitor
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("CellChangeMonitor", Probe.EndUs(__state)); }
    }

    [HarmonyPatch(typeof(PropertyTextures), "Sim200ms")]
    internal static class P_PropertyTextures
    {
        internal static void Prefix(out long __state) { __state = Probe.Begin(); }
        internal static void Postfix(long __state) { Probe.Record("PropertyTextures.Sim200ms", Probe.EndUs(__state)); }
    }
}
