# audio-hack-lab 实验报告：进程 loopback 抓取点与"防双声"静音方案

日期：2026-10-23 · 机器：MODENC\cheng · Windows 10 Pro 26H2 (26300.9711) · 非管理员 shell
实验代码：`tools/audio-hack-lab/lab`（独立 crate，不在 workspace 内）；脚本 `e3.sh` `e4a.sh` `e4b.sh`；日志 `*.log`

## 目标

Moiren 用系统 API（进程 loopback）拿到某应用的音频流并转发后，需要把该应用原来的输出链路静音，
避免"原始播放 + Moiren 转发副本"同时发声。核心问题：**抓取点（tap）在引擎管线里位于哪一级
音量/静音之前还是之后**。

## 实验结论（全部实测验证）

音频引擎管线顺序（26300 实测钉死）：

```
应用流 → [session 音量/静音(每应用)] → ✂进程loopback抓取点 → ✂经典loopback抓取点 → [端点主音量/静音] → 扬声器
                                        （汇入端点混音后、端点音量前）
```

| 实验 | 操作 | 捕获流结果 | 结论 |
|------|------|-----------|------|
| E3 | procloop 捕获 tone + 中途 session SetMute(true) | 静音后捕获流立即 -120dB 全哑，取消后恢复 | **抓取点在 session 静音之后** |
| E4a | procloop 捕获 tone + 中途端点主静音 | 静音窗口内信号纹丝不动（-13.6dB 恒定） | **抓取点在端点静音之前** |
| E4b | 经典 loopback + 中途端点主静音 | 同样不受影响 | **经典 loopback 抓取点也在端点静音之前**（26300 新引擎行为；旧系统会被掐死） |
| E6 | exclude 模式（捕获除目标外一切） | 正确排除，真静音、帧仍流动 | exclude 模式可用（采系统声排除自身） |
| 探针 | OpenProcess(audiodg, VM_WRITE\|VM_OPERATION\|CREATE_THREAD) | DENIED 0x80070005（VM_READ 也拒） | audiodg 为 PPL，注入路线死路 |

## 方案裁决

**✅ 方案 A（推荐，已验证）：进程 loopback 捕获 + 端点主静音 + Moiren 渲染到另一端点**

- `IAudioEndpointVolume::SetMute(true)` 静音原端点（扬声器不再出声 = 消灭双声）
- 捕获流不受影响（E4a 实证），Moiren 把处理后的副本渲染到别的端点
- 无需管理员、瞬时生效、可逆（记录原状态，结束时恢复）
- 粒度：整个端点全局静音（恰好符合"路由期间避免系统音频继续播放"的需求）
- 限制：Moiren 不能渲染回同一个被静音的端点

**✅ 方案 B（备选）：默认设备重路由到虚拟汇集**
- 默认渲染端点切到已签名的虚拟设备（本机有 VB-Cable `CABLE Input`、SteelSeries Sonar 虚拟端点）
- 应用跟随默认设备流入虚拟线，Moiren 从 `CABLE Output` 捕获端点采集，渲染到真实扬声器
- VoiceMeeter/Sonar 同款架构；无需任何静音。缺点：跟随默认设备的应用才生效、切换有毛刺

**❌ 被排除：session 静音（分应用静音）**——E3 证明会同时掐断捕获流。引擎设计如此，
任何"只静音目标应用但继续捕获它"的用户态组合都不存在（session 级是唯一的分应用音量点）。

**❌ 被排除：GFX/SFX 零输出 APO（注册 FxProperties，Equalizer APO 路线）**——效果半径与端点静音相同
（整个端点），却要管理员 + 注册表 + 重启 audiosrv + 自写 COM DLL，纯劣化，无必要。
（调研备注：FxProperties 的 `,2`=GFX/`,5`=SFX/`,6`=MFX/`,7`=EFX，新式端点有 SFX/MFX 时 LFX/GFX 被忽略；
Equalizer APO 证明无驱动注册第三方 APO 在 Win11 可行，将来若需要"系统内 DSP"可走这条路。）

**❌ 被排除：注入 audiodg 劫持**——PPL 保护，OpenProcess 写权限全拒。

## Moiren 集成建议

1. `moiren-windows-audio` 的 `process_loopback`（activation.rs/stream.rs）已具备正确的激活与
   Initialize 参数（本次实验顺带验证：缺 `IAgileObject` 的 handler 会得到 0x80070002，
   Initialize 必须用 `LOOPBACK|EVENTCALLBACK|AUTOCONVERTPCM|SRC_DEFAULT_QUALITY` + duration 0，
   否则 0x88890021 —— Moiren 现有实现已正确处理这两点）。
2. 新增端点静音控制器（`windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume`）：
   - routing 开始：`GetMute()` 记录原状态 → `SetMute(true)`
   - routing 结束/停止：恢复记录的状态
   - 注册 `IAudioEndpointVolumeCallback` 监听用户手动改动（策略：尊重或重设）
   - 崩溃残留问题：启动时若发现"被我们静音但未路由"的状态需提示恢复（或仅提示）
3. 渲染端点选择：routing 的输出必须 ≠ 被静音端点；catalog 里选非默认渲染端点（本机可用
   CABLE Input / Sonar 虚拟端点 / HDMI）。
4. 崩溃安全：控制侧落盘"静音屏障"状态（Moiren 本就有控制面 retired-plan 机制可挂）。

## lab 工具速查

```
lab default                    # 默认渲染端点
lab tone --dur 16 --freq 997   # 发测试音（打印 PID）
lab procloop --pid P [--exclude] --dur 12   # 进程 loopback 捕获（250ms 时间线: rms/Goertzel）
lab loopback --dur 12          # 经典 loopback 捕获
lab sessions [--pid P]         # 端点上的会话（PID/音量/静音）
lab mute --pid P --on|--off    # 分应用 session 静音
lab epmute --on|--off|--query  # 端点主静音
lab openproc                   # audiodg PPL 探针
```

测试期间扬声器状态：实验前端点静音=true（用户原状态），已恢复 true。

---

# 第二轮（同日追加）：隐藏 ABI 逆向 + 零依赖路由

## 已破解的隐藏面（全部实测）

1. **`Windows.Media.Internal.AudioPolicyConfig`**（Cloaked WinRT 静态类，Settings 背后的引擎）
   - 激活：`RoGetActivationFactory(L"Windows.Media.Internal.AudioPolicyConfig")`
   - 26300 上的接口是**新变体 IID `{AB3D4630-E242-459F-B02F-541C70306324}`**（EarTrumpet 的 21H2 版是 AB3D4648，SoundSwitch 三版皆不认识 → 必须动态 `GetIids` 取最后一个 IID 再 QI）
   - vtable：**Set=槽25、Get=槽26**（与 Get 的机器码序言逐字节同形可作校验）
   - `SetPersistedDefaultAudioEndpoint(pid, eRender, role, HSTRING)`——26300 起设备 ID 必须传**长格式** `\?\SWD#MMDEVAPI#{0.0.0.00000000}.{guid}#{e6327cad-dcec-4949-ae8a-991e976a79d2}`（短格式 `{0.0.0.00000000}.{guid}` 返回 E_INVALIDARG；Get 返回的也是长格式）
   - 生效语义：**即时更新引擎缓存 + 写注册表**（HKCU\...\Multimedia\Audio\DefaultEndpoint\<hash>_0，哈希由引擎算），对该 EXE 的**新进程/新流**生效，无需管理员、无需重启服务
2. **per-app 偏好存储**：键名=EXE NT 路径的 32 位哈希（算法未破，crc32/fnv/djb2/sdbm/murmur3/xxh32/rtl 系全不匹配）；纯注册表写入不生效（audiosrv 内存缓存为准）
3. **私有导出**（audioses.dll，序号导出）：`ActivateApplicationLoopback`(1) / `FromPID`(2) / `ForProcessTree`(3) / `AudioHistoryClient`(4) / `ForExcludedProcessTree`(7) / `WithOptions`(8) / `FromPIDWithOptions`(9)——WithOptions 家族未逆向
4. **引擎内部 Application Submix 系统**（PDB）：`CProcessSubmix`/`ISubmix`/`ConnectToRightSubmix`/`GetApplicationSubmixContextFor{Pid,ProcessTree,Excluded,Aumid}_Rpc`——引擎原生支持按应用拆混音并接/断到端点，是终极内部 ABI（未逆向）
5. `VAD\Process_Loopback` 是虚拟设备内部命名空间，全家族仅此一员；audiodg PPL 不可注入

## E13-E16 关键实验结论

| 实验 | 结果 |
|------|------|
| E16-1 隐藏 API 路由到活动端点(CABLE) | ✓ 新进程会话即时落到目标端点，其他应用不受影响 |
| E16-2 路由到 NOTPRESENT 幽灵端点（长格式） | API 接受；应用流被"黑洞"：不在任何活动端点（扬声器无双声✓），但 **procloop 也拿不到数据**（管道未建立，tap 无载体）→ 只能静音不能捕获 |
| E13/E16 procloop vs 端点静音 | 再次确认捕获不受端点静音影响 |

## 最终方案矩阵（全部零第三方依赖）

- **A（推荐，全实测）**：procloop 捕获目标应用 + `IAudioEndpointVolume::SetMute(true)` 静音原端点 + Moiren 渲染到真正的目标设备。粒度=整端点。
- **B（per-app 粒度，实测路由）**：`AudioPolicyConfig::SetPersistedDefaultAudioEndpoint`（长格式）把目标应用引到第二个**活动**端点 + 经典 loopback 从该端点采集 + Moiren 渲染到目标设备。前提：机器存在第二个活动输出（路由的定义本身就要求有第二个输出设备，通常成立）。
- **C（只想静音某应用）**：路由到幽灵端点（NOTPRESENT，长格式 ID）——应用被无声吞掉。不能捕获。
- 未竟：audioses `WithOptions` 私有导出、submix RPC——按需再启。

## 事故记录（透明化）

槽位探针误触发 `ClearAllPersistedApplicationDefaultEndpoints` 清空了 6 条用户偏好；已凭会话内完整注册表记录逐值恢复（restore_prefs.ps1），并确认键值一致。另：设置 UI 自动化过程中误触解除了端点静音并改了音量，已恢复 volume=0 + mute=true（用户原始状态）。
