# ONI-SimDLL_Rust
The code comments are in Chinese. Well… make do with it.
The latest steam: https://steamcommunity.com/sharedfiles/filedetails/?id=3779276157

A Rust reimplementation of Oxygen Not Included's native simulation DLL, targeting **U60 (build 764025)**.
Physics is bit-exact with the stock simulation; parallelism is region-level (one discovered world = one work unit, packed with LPT by area), so it needs no halo exchange and never changes simulation results.

Performance comparison, late-game colony, Moono alone vs Moono + this mod (Chinese narration):
[缺氧优化：Moono 与 SimDLL_Rust](https://www.bilibili.com/video/BV1qfyvzREVi/)

> Moono alone gets that colony to ~60 fps; this mod adds on top of it. Note that most of the gain in that video is Moono's — see the numbers below for what this mod alone contributes.

## Test machine

| | |
|---|---|
| CPU | AMD Ryzen 7 9800X3D (8 cores / 16 threads, 96 MB 3D V-Cache) |
| GPU | AMD Radeon RX 9070 XT |
| RAM | 32 GB |
| Display | 1920×1080 @ 240 Hz, vsync on |
| OS | Windows 11 (build 26300) |
| Game | ONI U60, build 764025 |

## Measured on this machine

Late-game colony, 12 discovered worlds, 143,456 simulated cells. Numbers are wall time of one simulation step (`UpdateData`), same save, same camera position:

| threads | sim ms/frame | speedup vs 1 thread |
|---|---|---|
| 1 | 18.6 | 1.00× |
| 4 | 8.2 | 2.27× |
| 7 | 8.8 | 2.11× |

The ceiling is **total area / largest world area** — worlds cannot be split, so the largest one is always the critical path. For this save that ceiling is **3.27×**, regardless of core count.

**Note that 7 threads is slower than 4.** The simulation is memory-bandwidth bound, not compute bound; past roughly half the physical cores the per-thread throughput drops faster than the parallelism helps. `ReservedCores` in `config.json` (1–4, default 1) controls this: threads = physical cores − reserved.

**On this machine the simulation is ~4–6 % of frame time** (≈ 200 ms frames in the sampled colony). Even a perfect simulation would not raise the frame rate visibly here — the remaining 95 % is rendering, pathfinding and Unity. On slower CPUs, and on colonies with many worlds, the share is much larger and the gains are real.

Stage breakdown for the same colony (CPU µs/frame, 7 threads, `Enable Logging` on):

| stage | µs/frame |
|---|---|
| post-process main loop | 7,632 |
| gas pressure | 3,988 |
| temperature | 2,789 |
| emitters | 2,181 |
| liquid | 1,950 |
| radiation | 867 |
| disease growth | 868 |
| grid copy | 634 |
| disease diffusion | 460 |
| space vacuum | 196 |

## When it helps, and when it does not

- **Multi-world colonies** — one work unit per discovered world, up to the area-ratio ceiling above.
- **Single-world colonies** — effectively nothing: one region is one work unit, nothing to parallelise.
- **Fast CPUs** — the sim is a small fraction of the frame; expect fewer simulation-induced drops rather than a higher frame rate.
- **Camera panning hitches** — unrelated. Those come from the render path (batch rebuild when the visible area crosses a 32-cell chunk boundary); the sim runs on its own thread and does not block them.

## Known limitations

- Single-world saves get almost no parallelism.
- Thread counts beyond ~half the physical cores lose more to memory bandwidth than they gain from parallelism.
- Windows only in the current build (U60). The Linux build is not maintained.

## Install

Extract the `SimDLL_Rust` folder into `Documents\Klei\OxygenNotIncluded\mods\Local\`, then enable it in game. Or subscribe on the Steam Workshop.

## PerfProbe (development tool)

`SimDLL_PerfProbe-U60.zip` in the latest release. It is a diagnostics-only mod: it times a set of engine methods per frame, splits every sample by camera-moving / camera-still and by chunk-boundary crossing, and records frames over 10 ms with their in-frame breakdown.

Output goes to `mods\Local\SimDLL_PerfProbe\perf.log`, one line per 5 s window. Nothing is uploaded anywhere. Leaving it enabled for normal play is harmless but pointless outside measurement.

Patches are applied one class at a time and failures are skipped with a warning, so a game update that changes a signature degrades the probe instead of breaking the game.
