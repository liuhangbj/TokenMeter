# TokenMeter

跨平台菜单栏 App，一眼看清你所有 AI 平台的 token / 额度用量。

macOS 菜单栏 / Windows 任务栏托盘常驻，点击展开下拉面板，各平台额度、余额、重置倒计时一屏打尽。

## 功能

- **托盘下拉面板**：额度条 + 余额 + 重置倒计时，5 段警示色（绿→橙→红）一眼识别紧张度
- **明暗主题**：跟随系统自动切换
- **卡片排序**：上下箭头自定义顺序，持久化记忆
- **自适应高度**：内容少时紧凑，多了才滚动
- **添加供应商向导**：API Key 表单 / OAuth 浏览器授权 / 本机 CLI 凭证一键导入
- **凭证安全**：AES-256-GCM 加密存储，token 过期自动刷新
- **后台刷新**：间隔可调（1–30 分钟），打开面板立即刷新
- **开机启动**：可选
- **零数据库**：除配置文件外不产生本地数据，用量曲线点"查看详情"跳官方控制台

## 支持平台

| 平台 | 类型 | 数据来源 | 状态 |
|------|------|---------|------|
| OpenAI Codex | 订阅（5h/7d 额度） | OAuth / 本机 CLI | ✅ |
| OpenAI Platform | 按量（花费 + token 用量） | Admin API Key | ✅ |
| Kimi Code | 订阅（5h/周/Extra Usage） | 设备码 OAuth / 本机 CLI | ✅ |
| Moonshot | 按量余额 | API Key | ✅ |
| DeepSeek | 按量余额 | API Key | ✅ |
| 腾讯 TokenHub 按量 | 按量 token 用量 | SecretId/Key | ✅ |
| 腾讯 Token Plan（个人版） | 订阅 | 无官方查询 API | 🔜 待官方开放 |
| 腾讯 Coding Plan | 订阅 | 无官方 API | 🔜 待官方开放 |

### 路线图（计划接入）

| 平台 | 备注 |
|------|------|
| Claude (Anthropic) | 订阅 + API 按量 |
| Gemini (Google) | AI Studio API + Advanced 订阅 |
| GLM (智谱) | bigmodel.cn API |
| MiniMax | 海螺 API |
| Ali Qwen (通义千问) | 阿里云百炼 API |
| OpenRouter | 聚合网关余额 |
| 豆包 (字节) | 火山引擎方舟 API |
| 小米 MiMo | 小米大模型 API |

> 各平台接口调研与字段确认记录见 [docs/01-provider-matrix.md](docs/01-provider-matrix.md)。欢迎 PR 补充。

## 安装

从 [Releases](https://github.com/liuhangbj/TokenMeter/releases) 下载：

| 平台 | 文件 |
|------|------|
| macOS (Apple Silicon) | `TokenMeter_x.x.x_aarch64.dmg` |
| Windows (MSI) | `TokenMeter_x.x.x_x64_en-US.msi` |
| Windows (NSIS) | `TokenMeter_x.x.x_x64-setup.exe` |

> ⚠️ 当前版本未做代码签名：macOS 首次打开需在「系统设置 → 隐私与安全性」允许；Windows 可能提示 SmartScreen，选"仍要运行"。

## 开发

技术栈：**Tauri 2 + Rust + React + TypeScript + Vite**

```bash
# 依赖：Rust (rustup)、Node.js 22+
npm install
npx tauri dev        # 开发模式（热更新）
npx tauri build      # 构建安装包
```

### 本机测试构建 / 部署

```bash
scripts/dev-deploy.sh               # debug 构建 → 部署 → 直接启动 App
scripts/dev-deploy.sh --install     # 构建并安装到 ~/Applications/TokenMeter Dev.app
scripts/dev-deploy.sh --release     # release 构建
scripts/dev-deploy.sh --no-run      # 只构建不启动
scripts/dev-deploy.sh --isolate     # 独立数据目录，不影响正式版凭证/设置
```

- 产物：`src-tauri/target/<profile>/tokenmeter` 与 `.../bundle/macos/TokenMeter.app`
- 启动日志：`/tmp/tokenmeter-dev.log`；停止：`kill $(cat /tmp/tokenmeter-dev.pid)`
- 本机没有签名私钥时自动跳过 updater 签名产物；正式发布仍走 GitHub Actions
- 测试前请先退出正式版 TokenMeter（单实例锁会让第二个实例直接退出）

### 架构（三层）

```text
src/                         # UI 层：React + TypeScript（平台无关）
├── App.tsx                  # 托盘面板（单窗口，含内嵌添加供应商向导）
├── ProviderCard.tsx         # 平台卡片
└── SettingsPanel.tsx        # 设置面板

src-tauri/src/
├── core/                    # Core 层：平台无关核心（不依赖 Tauri）
│   ├── providers/           # 8 平台 provider（统一 Provider trait）
│   ├── store.rs             # AES-256-GCM 加密凭证存储
│   ├── scheduler.rs         # 定时/触发刷新（并发 + 失败可见）
│   ├── scheduler_ctl.rs     # 刷新间隔广播 + 立即刷新信号
│   ├── settings.rs          # 设置文件持久化
│   └── oauth_codex.rs       # Codex PKCE / Kimi 设备码 OAuth
├── platform/                # Platform Shell：平台差异集中地
│   ├── tray.rs              # 托盘/菜单栏（macOS 顶部、Windows 任务栏）
│   └── mod.rs               # macOS Accessory 策略、系统浏览器打开
├── commands.rs              # IPC 薄层（前端 ↔ core/platform）
└── main.rs                  # 组装根：插件、窗口事件、退出守卫
```

### 设计原则

- **三层边界**：Core 不感知 Tauri；平台差异（托盘、焦点、Dock、浏览器）只允许出现在 `platform/`；UI 只通过 commands 与后端对话
- **单窗口**：全 App 只有一个 WebView（托盘面板），添加供应商/设置都是面板内视图——规避 Windows WebView2 多窗口白屏/冻结
- **零数据库**：不存历史、不做统计，实时拉取实时显示，用量曲线跳官方控制台看
- **凭证不落明文**：AES-256-GCM 加密，随机主密钥 0600 独立存放（无签名环境的现实折中）
- **数据驱动 UI**：provider 声明 `auth_spec`，向导表单自动渲染，加新平台只需实现一个 trait

## License

[MIT](LICENSE)
