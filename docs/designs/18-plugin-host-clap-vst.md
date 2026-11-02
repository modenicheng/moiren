# 第三方插件宿主:CLAP / VST2 / VST3

> 状态:Implemented (engine + crate) / 2026-11-02
> 范围:`crates/moiren-plugins`(格式后端)+ `moiren-engine` 集成(节点、绑定、参数)。
> 应用层(扫描服务、插件浏览器 UI、inspector 面板、i18n)为后续项,见文末。

## 1. 概述

Moiren 以"万物皆节点"的方式接入第三方插件:一个插件就是一个 `NodeKind::Plugin`
节点,主输入/输出总线等宽(节点单宽度),参数通过引擎既有的
ParamSpec → ControlPort → ParameterRuntime → ProcessParameters 管线自动化。

```
crates/moiren-plugins        ← 叶子 crate:格式 ABI + 扫描 + 实例(无引擎依赖)
    ▲
crates/moiren-engine         ← PluginProcessor: RtProcessor 适配器 + 节点装配
    ▲
crates/moiren-app            ← (后续)扫描服务 / 浏览器 UI / 持久化
```

依赖方向单向(engine → plugins),避免循环:`PluginInstance` trait 是插件
crate 的自有抽象,不引用引擎类型。

## 2. moiren-plugins 的结构

| 模块 | 职责 |
|---|---|
| `spec.rs` | `PluginDescriptor` / `PluginParameterSpec` / `PluginError`(serde 持久化) |
| `instance.rs` | 格式无关的 `PluginInstance` trait:`process` 是唯一 RT 入口 |
| `module.rs` | 按规范化路径去重的库加载注册表;CLAP init/deinit 与库生命周期配对 |
| `clap/` | CLAP 1.2 ABI(`abi.rs`)与后端;宿主不实现任何扩展 |
| `vst2/` | VST 2.4 AEffect ABI 与后端;audioMaster 只应答版本/采样率/块长/进程级 |
| `vst3/` | VST3 COM 接口镜像、宿主 COM 对象(引用计数不触发删除)、后端 |
| `scan.rs` | 标准目录(CLAP_PATH/VST3_PATH/VST_PLUGIN_PATH + Common Files 等)、递归扫描、VST3 bundle 解析 |

### 2.1 实时约束

`PluginInstance::process` 的宿主侧零分配:指针表、事件队列、VST3
IParameterChanges/IParamValueQueue 容器全部在实例化时预分配,每块只写字段。
参数变化按块差分(与 last_pushed 比较),只推送变化项,上限 128 事件/块。
VST2 没有块内参数事件,按经典宿主方式在 processReplacing 前调 setParameter。
插件自身的 RT 行为是插件作者的责任。

### 2.2 生命周期

- CLAP:entry.init(路径) → factory → create_plugin → plugin.init →
  audio-ports/params/latency/state 扩展 → activate → start_processing;
  Drop 时 stop_processing → deactivate → destroy;init/deinit 随库句柄引用计数配对。
- VST2:VSTPluginMain → effOpen → setSampleRate/BlockSize → 读参数默认值(此时
  参数即默认)→ effMainsChanged(1) → effStartProcess;状态块走
  effGet/SetChunk(program),加载时挂起/恢复。
- VST3:工厂 → createInstance(IComponent) → queryInterface(IAudioProcessor/
  IEditController)→ initialize(IHostApplication)→ 主总线激活(其余 audio 总线
  显式停用)→ setupProcessing → setActive → setProcessing;状态为宿主自定义分帧
  `[u32 len][component][u32 len][controller]`,回放时同时喂
  setComponentState 与 setState。

### 2.3 测试策略:交叉验证

`tests/fixtures/gain_plugin.rs` 是一个自包含 gain 插件(rustc 直接编译为
cdylib),同时导出三格式入口。它与宿主侧 ABI 转写相互独立——两份独立转写
逐字节一致,布局/vtable 顺序错误才可能暴露。这套测试在落地过程中实际抓到过
fixture 侧的接口指针偏移错误(VST3 多接口对象的 this 调整),证明交叉有效。

## 3. 引擎集成

- `NodeKind::Plugin`(moiren-core):1 入 1 出,通道数 = 插件主总线宽度。
- `NodeConfig::Plugin(PluginSettings)`:描述符 + 扫描探针采集的参数表
  (格式自身单位:CLAP 明文值、VST2/3 归一化 0..1),一一映射到
  `ParamDomain::Float`。
- `Binding::Plugin(PluginProcessor<S>)`:内核与其参数模式必须成对进入计划
  (`Builder::prepared` 模式),故不能复用类型擦除的 `Binding::Io`。
- `PluginProcessor`(processor/plugin.rs):
  - `transport_interchangeable() == false` ⇒ 编译器永不 InPlace 折叠
    (plan.rs 的 in_place_candidate 门),插件始终拿到独立 in/out 缓冲;
  - f32 平面暂存桥接引擎采样类型(f32/f64 都可托管插件);
  - `validate_parameters` 要求参数表全部在运行时表中;
  - 处理失败的块清零输出(不撕裂整图)。

参数初值即插件默认;录制意图(ParameterTarget::Node)照常覆盖默认,重编译后的
新实例通过运行时参数管线收到当前值。

## 4. 设计决策与理由

1. **叶子 crate + 引擎适配器**:打破循环依赖的最小方案。
2. **等宽主总线才可托管**:节点单宽度模型的直接推论;synth(0 输入)与不等宽
   总线在实例化时报 `NoAudioPorts`,不静默降级。
3. **按块参数差分**:与引擎 FloatRamp 语义一致(常量斜坡取目标值),避免每块
   全量推送。
4. **VST3 宿主对象引用计数不触发删除**:对象随实例释放;插件在 terminate 前
   释放其全部引用是格式契约。
5. **VST2 按公开 ABI 重实现**(无官方 SDK 分发):转写自公开的 aeffectx.h 接口
   与 rust-vst 参考;许可情况在 crate 文档(lib.rs)中注明。

## 5. MVP 排除项(后续工作)

- GUI 编辑器、sidechain、MIDI、bypass 参数特判、sample-accurate 参数曲线
- 宿主扩展:CLAP thread-check/logging、VST3 IPlugView/IHostApplication 派生
  接口、重启请求(restartComponent/request_restart 目前为确认即忽略)
- 跨 plan 实例复用(引擎 with_reuse 仍限 Source/Sink;重编译=重新实例化,
  插件内部状态不迁移——参数意图除外)
- 崩溃隔离(第三方插件可直接拖垮进程)
- 应用层:扫描服务、插件浏览器 UI、inspector 参数面板、i18n、项目持久化
  (GraphNodeKind 已带 `plugin` 序列化变体;编辑器暂不能创建插件节点,
  preparation 会给出明确错误)

## 6. 验证

- `cargo test -p moiren-plugins`:5 个 FFI 集成测试(三格式扫描/实例化/
  处理/参数/状态往返)。
- `cargo test -p moiren-engine --test compiler`:插件节点经编译器渲染立体声
  gain 固定块。
- 全 workspace fmt / clippy(-D warnings)/ test 通过(见 CI)。
