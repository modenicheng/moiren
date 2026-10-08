# W00 动态设备与驱动管理实验

用户指定：动态创建 / 删除 endpoint，以及驱动管理；确认没有支持动态端点的已签名 INF 包后，先完成 PnP 生命周期与权限测试。沿用 QQMusic 播放设置不变、统计记录、W00 可行性实验范围。

**Goal：**实测 SwDevice 的权限、异步创建、handle 生命周期、同 identity 再创建、PnP 移除与精确实例卸载；分开报告普通软件节点和真实音频 endpoint。

**Architecture：**独立 `swdevice` 模块和 opt-in example，复用 COM / HANDLE guards 与只读目录。固定 MoirenW00 enumerator、运行时 GUID identity、不匹配已安装音频驱动的测试 hardware ID。callback context 保留至 SwDeviceClose 返回；本次创建句柄先关闭，再等待 not-present，最后用 SetupAPI / DiUninstallDevice 清理确切实例。没有接受外部 device ID 的卸载入口。

**约束：**不安装 / 更新 / 删除驱动包，不改 test-signing / Secure Boot，不重启，不修改已有 device、默认角色或音量。仅创建本次临时测试节点。DriverRequired 场景验证缺少匹配驱动，不替代 PCM 引擎；W15 完整验收保持未完成。管理员运行通过 Windows UAC，普通权限拒绝也形成有效记录。

- [x] 纯 guard 测试拒绝 ROOT / MMDEVAPI / 相似 namespace / 错误 identity；本地 callback 测试不触碰真实设备。
- [x] 编译仅创建固定 namespace 的探针；非管理员实际调用并记录 HRESULT，不提前用权限判断代替 API 实测。
- [x] 管理员分别测试 raw software node 与 DriverRequired node，记录 callback、presence、problem code、INF binding、默认 lifetime、同 ID 第二次 create。
- [x] 默认 Handle lifetime，每轮完整创建 / 关闭 / 精确卸载后复用同 identity，共三轮；关闭后等待不 present，记录 phantom 状态；精确卸载后再次查询，不将 close 返回等同删除完成。
- [x] 创建期间和结束时枚举所有状态的 audio endpoint；前后对比 QQMusic session、默认角色及 endpoint 设置。
- [x] package fmt / check / tests / Clippy；保存实验结果，登记真正动态音频 endpoint、驱动安装升级卸载、PCM transport 与签名尚未测试。

结果见 [2026-10-08 实验记录](../experiments/windows/2026-10-08-w00-swdevice.md)。普通权限 create 被拒绝；管理员六个完整循环通过。没有测试 close 后跳过卸载直接 re-create、ParentPresent、强制终止进程或普通权限对已存在节点的卸载调用。
