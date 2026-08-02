# 三层架构（2026-08-02 重构）

目标：一套代码双平台可维护——Core 与 UI 跨平台共享，平台差异收敛到薄壳。

## 分层规则

| 层 | 位置 | 职责 | 允许依赖 |
|---|------|------|---------|
| UI | `src/` | 渲染、交互、面板内路由 | 只通过 Tauri commands 调用后端 |
| Core | `src-tauri/src/core/` | provider 抓取、凭证加密、调度、设置、OAuth | 纯 Rust 生态（reqwest/chrono/serde），**禁止 tauri*** |
| Platform Shell | `src-tauri/src/platform/` | 托盘、系统浏览器、macOS Dock 策略 | tauri + cfg(target_os) |
| 组装/IPC | `src-tauri/src/main.rs`、`commands.rs` | 插件注册、窗口事件、命令转发 | 全部 |

\* 唯一例外：`scheduler` 通过 `notify` 回调（`impl Fn()`）把"数据已更新"交给 platform 层发事件。

## 关键约定

1. **单窗口**：全 App 只保留 popover 一个 WebView。添加供应商、设置均为面板内视图。
   Windows WebView2 在运行时创建/销毁第二个 WebView 会出现白屏（页面不执行 JS）
   和面板冻结，单窗口从根上消除这一类问题。
2. **平台差异只出现在 `platform/`**：
   - 托盘定位/点击防抖/失焦守卫（Windows 与 macOS 行为不同）
   - 系统浏览器打开（`open` vs `cmd start`）
   - macOS `ActivationPolicy::Accessory`（隐藏 Dock）
3. **构建正确性**：所有独立二进制必须启用 `custom-protocol` feature，
   否则 release 构建也会按 dev 模式连 `devUrl`（localhost:1420）。
   - CI（smoke.yml）：`cargo build --release --features custom-protocol`
   - 本机脚本（dev-deploy.sh）：`tauri build --features custom-protocol`
4. **数据与设置**：凭证 `credentials.json` + 随机主密钥 `credentials.key`（0600），
   设置 `settings.json`，都在同一数据目录；可用 `TOKENMETER_DATA_DIR` 覆盖（测试隔离）。

## 未来拆分路径

若后续决定 macOS / Windows 分开维护：

1. 把 `core/` 抽成独立 Rust workspace crate（已无 tauri 依赖，成本最低）；
2. `platform/` + `commands.rs` + `main.rs` 按平台拆成两个壳；
3. UI 层继续共享（React），平台特有交互用条件编译/环境变量收敛。
