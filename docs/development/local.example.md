# 本地环境模板

复制本文件内容到未跟踪的 `docs/development/local/README.md`，再按下面的文件拆分填写真实信息。**不要提交真实机器、路径、ssh 别名、代理或凭据。**

## 推荐目录

```text
docs/development/local/          # gitignored
  README.md                      # 索引 + 备份说明
  preferences.md                 # 个人协作偏好
  machines.md                    # 本机/测试机/网络
  local-quirks.md                # 本机 cargo/GPUI/Miri/测试坑
  hardware.md                    # 个人硬件与验收记录
  obs-target.json                # 专用 OBS 安装与采集端点
```

## 填写原则

- 只写“这台机器/这个人”的信息，不写通用规则；通用规则在 `docs/development/`。
- 不写私钥、令牌、密码、恢复码；ssh 配置留在 `~/.ssh`，这里只写别名。
- 用占位符代替真实地址/姓名；真实值只存在于本地文件。
- 备份到私有仓库/私有 gist/私有云盘；不要在公开仓库里用 submodule 指向私有库。

## 可填写项（示例占位符）

### preferences.md

- 沟通语言/风格：`<...>`
- 决策偏好：`<...>`
- UI/产品偏好：`<...>`
- 提交/授权习惯：`<...>`
- 机器负载与异步习惯：`<...>`

### machines.md

- 本机：`<OS / 仓库绝对路径 / worktree 目录 / target 策略>`
- 测试机：`<别名 / 主机 / 仓库路径 / remote 配置 / 环境变量 / 临时目录>`
- 网络：`<代理地址 / SSH 回退 / HTTPS 回退 / 其他工具链镜像>`
- 跨机流程：`<push/ssh/后台任务/哨兵文件>`

### local-quirks.md

- cargo 并发/路径/被运行中 exe 锁住：`<...>`
- Miri 本机误报：`<...>`
- 时序脆弱测试：`<...>`
- worktree/git 本机坑：`<...>`

### hardware.md

- 主机/接口/监听设备：`<...>`
- 实机验收清单与结果：`<...>`
- soak 数据位置：`<...>`

### obs-target.json

`scripts/obs-reuse.py --check` 和 OBS 采集会读取脚本所在仓库的
`docs/development/local/obs-target.json`。如果单独部署测试脚本，用
`MOIREN_OBS_TARGET_PROFILE` 指定本地配置的绝对路径。配置为 UTF-8 JSON，
以下示例的值都需替换。

```json
{
  "computer_name": "<Windows 计算机名>",
  "user_sid": "<运行测试的 Windows 账户 SID>",
  "installation_root": "<专用 OBS 安装根目录的绝对路径>",
  "executable_sha256": "<bin/64bit/obs64.exe 的 SHA-256>",
  "capture_endpoint_id": "<Windows 音频渲染端点的完整 ID>"
}
```

`--check` 只检查目标身份、可执行文件哈希、OBS 进程和 WebSocket 端口。
采集使用配置中的端点 ID，不能选择系统默认端点。

## 本地代理记忆

- zcode 记忆目录：`<...>`
- 计划目录：`<...>`
- SDD 证据目录：`<...>`
- 更新时机：战役收官、审计定案、踩坑定位、用户拍板之后。
