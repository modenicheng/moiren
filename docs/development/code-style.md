# 代码风格与测试写法

本文管 Rust 源码与测试的写法；UI 的交互/视觉规则见 [app-ui.md](app-ui.md) 与 [FRONTEND.md](../../FRONTEND.md)，提交格式见 [workflow.md](workflow.md)。

## 格式与命名

- 四空格缩进，rustfmt 默认配置（仓库没有 `rustfmt.toml`）。提交前跑 `cargo fmt --all -- --check`。
- `snake_case` 模块/函数/变量，`PascalCase` 类型，`SCREAMING_SNAKE_CASE` 常量；实时线程相关类型沿用 `Rt*` 前缀，且只用于真正跑在音频线程的类型。
- 模块按职责拆分；避免 God 文件。拆文件是为可读性与边界，不是为行数。
- 可见性最小化：默认私有/`pub(crate)`，只有跨 crate 契约才 `pub`；`pub` 面变化要当成 API 变化审查。

## unsafe、COM 与实时注释

- 每处非显然 `unsafe` 必须有 `SAFETY` 论证（为什么满足安全前提、生命周期/线程/所有权由谁保证）。
- COM 对象要注明所属线程与释放时机；回调路径要注明能否阻塞、是否在音频线程。
- 实时约束不写在类型系统里，必须在代码/模块头注释中显式说明：哪些函数是 RT 入口、允许什么、禁止什么。
- 平台相关代码用 `#[cfg(windows)]`/feature 门控，并保留非 Windows 的编译面（CI 会做 Linux check）。

## 错误与日志

- 库层用 `thiserror` 定义可匹配的类型化错误；错误链保留上下文，不把面向用户的句子硬编码进库。
- 音频线程禁止构造/分配错误字符串；错误用预分配的 ID/枚举/句柄带回控制面再格式化。
- 运行期失败用 `log` 宏并带 `target: "moiren::<domain>"`（如 `moiren::audio::service`、`moiren::app`）；只有 logger 初始化前的启动失败可以用 `eprintln!`。不要提交调试用 `println!/eprintln!`。
- 面向用户的错误/状态要结构化（GraphFault 等）并走 i18n；不要用 `format!("{:?}")` 直接进 UI。

## i18n 与用户可见文本

- `moiren-app` 的所有用户可见文本走 `t!`，键集以 `crates/moiren-app/locales/zh-CN.yml` 为唯一事实源；`tests/i18n_ledger.rs` 与 `src/i18n_guards.rs` 会拒绝漂移。
- 完整句子照常汉化；音频术语简称（M/S、ST、MONO、SWAP、L/R、n CH、DIM 等）所有语言恒等英文，不做汉化。
- 新增/改文案必须同步 catalog 的 `en`/`zh-CN` 双字段与 `%{placeholder}` 集合；不要在代码里拼 `t!` 的动态 key（除非在 `DYNAMIC_KEY_EXCEPTIONS` 中有据登记）。

## 测试

- 用 Rust `#[test]`；测试名描述行为，`snake_case` 长句（例如 `compatible_update_retains_delay_history_across_block_boundary`）。
- 回归测试放在受影响行为旁边：模块内 `#[cfg(test)] mod tests` 或同目录 `tests.rs`；跨 crate 黑盒测试放 `crates/<crate>/tests/`。
- 保留既有的分配、并发、图校验与 DSP 覆盖；`moiren-signal` 的分配断言与 barrier 并发测试是范本，RT 测试不要用 `sleep` 等待条件，用 deadline + 轮询/通道。
- UI headless 测试用 `--features gpui-test`；测试构造 shell 时必须清空 `autosave.path`/`preferences.path`（共享 harness 已处理），否则可能覆盖真实 `%APPDATA%\Moiren`。
- 不靠测试伪造观测数据；没有真实来源的读数用 Unknown/`—`。
- TODO/FIXME 不留在代码里：要么当场做，要么记入 [docs/TODO.md](../TODO.md)。

## 文档与注释

- 非显然的兼容性、实时约束、所有权设计、平台 quirk 要写注释；注释解释"为什么"，代码表达"做什么"。
- 不提交一次性调试文件、日志、截图产物；证据放 `docs/` 的规定位置（见 [documentation.md](documentation.md)）。
- 修改公共行为时同步更新模块头/README/设计文档；不要留下与行为矛盾的旧注释。
