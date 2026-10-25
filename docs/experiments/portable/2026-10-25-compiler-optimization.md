# Graph Compiler 优化基线

日期：2026-10-25。提交：`275f634`（Compiler 实现来自前一提交
`04c2fac`）。这份记录建立可重复的 portable CPU 基线，比较
`compile_reference()` 的逐 Edge 独立执行和 `compile()` 的 Bus 融合、last-use、
原位处理与 slot 复用。它只测已经准备完成的 Engine render，不包含编译、设备、
桥接、SRC 或物理端到端时延。

## 条件与方法

- Windows x64；Intel Core Ultra 9 285H，16 核 / 16 线程。
- `rustc 1.99.0 (b940084d7 2026-09-28)`，`--release`，无额外 feature。
- f64、stereo、48 kHz；输入数量为 1、8、32，block frames 为 16、64、256。
- 每个组合先 warm up 64 次，再交替测量 reference 和 optimized 各 5 轮；每轮调用
  1000 次 render，报告 5 轮的中位数 ns/render。
- Source 使用固定软件音源，Sink 只消费 planar buffer；没有打开 Windows 音频设备。
- `--ramps` 版本为每条 Edge 提交 gain 和 pan 的活动 ramp，仍使用同一矩阵。

复现命令：

```powershell
cargo run --locked --release -p moiren-engine --example compiler_optimization
cargo run --locked --release -p moiren-engine --example compiler_optimization -- --ramps
```

## 常量参数

| inputs | frames | operations ref/opt | slots ref/opt | ns/render ref/opt | speedup |
|---:|---:|---:|---:|---:|---:|
| 1 | 16 | 5 / 4 | 4 / 2 | 135 / 105 | 1.29x |
| 1 | 64 | 5 / 4 | 4 / 2 | 332 / 220 | 1.51x |
| 1 | 256 | 5 / 4 | 4 / 2 | 1043 / 704 | 1.48x |
| 8 | 16 | 19 / 11 | 18 / 9 | 595 / 301 | 1.98x |
| 8 | 64 | 19 / 11 | 18 / 9 | 1531 / 495 | 3.09x |
| 8 | 256 | 19 / 11 | 18 / 9 | 5394 / 1564 | 3.45x |
| 32 | 16 | 67 / 35 | 66 / 33 | 2274 / 1027 | 2.21x |
| 32 | 64 | 67 / 35 | 66 / 33 | 6047 / 1876 | 3.22x |
| 32 | 256 | 67 / 35 | 66 / 33 | 21621 / 6087 | 3.55x |

## 活跃 gain/pan ramp

| inputs | frames | operations ref/opt | slots ref/opt | ns/render ref/opt | speedup |
|---:|---:|---:|---:|---:|---:|
| 1 | 16 | 5 / 4 | 4 / 2 | 169 / 136 | 1.24x |
| 1 | 64 | 5 / 4 | 4 / 2 | 431 / 331 | 1.30x |
| 1 | 256 | 5 / 4 | 4 / 2 | 1461 / 1103 | 1.32x |
| 8 | 16 | 19 / 11 | 18 / 9 | 836 / 514 | 1.63x |
| 8 | 64 | 19 / 11 | 18 / 9 | 2299 / 1427 | 1.61x |
| 8 | 256 | 19 / 11 | 18 / 9 | 8769 / 5193 | 1.69x |
| 32 | 16 | 67 / 35 | 66 / 33 | 3090 / 1846 | 1.67x |
| 32 | 64 | 67 / 35 | 66 / 33 | 9309 / 5452 | 1.71x |
| 32 | 256 | 67 / 35 | 66 / 33 | 35691 / 19994 | 1.79x |

## 结论与使用方式

本机这次运行中，优化路径相对 reference 的观测范围为常量参数
`1.24x–3.55x`、活动 ramp `1.24x–1.79x`。优化同时把 32-input 场景的调度从
`67` 降到 `35` 个 operation，并把物理 slot 从 `66` 降到 `33`；这些结构统计比
纳秒值更适合作为跨机器回归检查。

这些数字是单台机器、单次记录，不是 CI 的硬性时限，也不代表设备 period、
异步 clock bridge、SRC、插件或物理端到端 latency。已有 Compiler 测试继续负责
逐位结果、variable-block、参数 ramp、RT 零分配和预算约束；重新运行上面的两条
命令时，应同时保留命令输出、提交号和机器条件。

后续 Compiler 优化应先保持 reference/optimized 逐位等价，再比较同一矩阵的
operation/slot 统计和 render 中位数。当前尚未完成的 Compiler 性能工作包括
f32 与更大 graph 的系统化基线、真实设备/bridge 的端到端 profiling，以及相邻
Gain/Pan 操作的进一步 kernel 融合；这些不应由本记录推断为已完成。
