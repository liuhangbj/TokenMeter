# 更新记录

## v0.9.7（2026-08-12）

### 新供应商

- 新增 Gemini Code Assist：支持 Google OAuth、本机 Gemini CLI 凭证导入、模型额度和 AI Credits
- 新增火山引擎 API：显示可用余额、现金、信用、冻结金额与欠费调整项
- 新增 SiliconFlow 国内站与国际站：显示总余额、充值余额和赠送余额

### 主题与视觉

- 建立“主题风格 × 明暗外观”契约，Classic、Claude、Cyber 均支持浅色、深色和跟随系统
- Claude 主题采用暖纸、陶土橙、Literata 书面字体和书签式会员标签；Cyber 提供完整科技光影、网格与字体体系
- 重绘应用图标、macOS 菜单栏图标和 Windows 彩色托盘图标，并统一小尺寸识别语言
- 主面板、设置、供应商向导和认证表单全部接入统一主题组件

### 更新与稳定性

- 自动更新改为后台定时检查和下载；下载完成后在固定标题区提示，由用户点击“升级并重启”
- 网络恢复后自动补检，失败后延迟重试；安装失败保留重试入口，不静默安装或擅自退出
- 主题和明暗设置持久化，并在首帧恢复，避免切换或启动时闪白

## v0.9.6（2026-08-09）

### 新供应商

- 新增 Claude 套餐浏览器 OAuth 与本机 Claude Code 凭证导入，支持套餐、额度窗口与 Extra Usage
- 新增 Anthropic API：通过 Admin API Key 查询组织月度成本、输入/输出 Token、缓存与 Web Search 用量
- 新增 OpenRouter OAuth / API Key 双入口，显示 Credits 余额、Key 预算与周期花费
- 新增 GLM Coding Plan，以及 GLM API 现金余额、今日花费与 Token/次数资源包
- 新增 MiniMax Token Plan，以及 MiniMax API 国际/国内站现金、代金券、Credit 与欠款余额

### 界面与契约

- Anthropic、GLM、MiniMax 等厂商统一按 Plan / API 双产品折叠展示，默认收起
- API Key 产品可声明本机 CLI / 环境凭证导入，不再与 OAuth 类型绑定
- 套餐主值统一显示剩余百分比；无余额 API 账户以本月花费作为主值
- Token 与请求数使用 K/M/B/T 紧凑格式，GLM 资源包按官方余额字段和 Token/次数单位展示
- 设置面板的检查更新区域增加当前版本号

## v0.9.5（2026-08-09）

- 修复 Windows 纯托盘模式下 Codex/Kimi 授权页无法打开；改由后端 ShellExecuteW 调用默认浏览器
- 浏览器打开失败不再终止设备码轮询，并提供重新打开、复制地址和手动选择地址的回退入口
- 导入已过期的 Codex CLI access token 时，自动使用 refresh token 续期并再次验证
- Codex refresh 请求改为当前官方 JSON 协议；轮换后的 token 安全同步回同账号 CLI `auth.json`
- CLI 已切换账号时不覆盖其凭证；网络错误与真正的凭证过期分别提示

## v0.9.4（2026-08-09）

### 界面与账号

- 精修明暗主题、卡片层次、字体、配色与窗口阴影，标题栏和设置区固定不随内容滚动
- 统一套餐额度与充值余额两套展示语言，修正 Codex Pro 5X、Credit、Kimi Extra Usage 等字段
- 支持同一平台添加多个账号，账号名称按“昵称 → 用户名 → 邮箱 → ID”显示
- 添加供应商改为按厂商折叠分组，Plan / API 类型由统一字段契约驱动

### 跨平台与稳定性

- 修复 macOS 窗口尖角阴影、默认尺寸与自适应高度
- 重写 Windows 托盘面板定位、隐藏窗口、首次点击和 DPI 缩放处理，并增加云端可见面板冒烟测试
- 修复删除账号、昵称保存、刷新间隔与卡片排序之间的状态覆盖问题
- 设置与凭证改为原子写入；损坏文件和异常密钥停止覆盖；旧私有文件权限自动收紧为 0600
- 修复 Kimi OAuth 在 Windows 上错误上报 macOS 设备型号的问题

### 更新与发布

- 自动更新改为后台检查并下载，只有用户确认后才安装并重启
- 修复 `latest.json` 指向草稿 Release 导致安装包 404 的问题
- 发布工作流必须等齐 macOS / Windows 签名，发布后自动验证元数据和两个下载地址

## v0.9.3（2026-08-03）

### 架构

- 重构为三层架构：平台无关的 core（providers / 凭证 / 调度 / 设置 / OAuth）+ platform 壳 + UI，双平台一套代码可维护
- 移除「添加供应商」独立窗口：添加供应商、设置全部内嵌进托盘面板（单 WebView），从根上规避 Windows WebView2 多窗口白屏/冻结问题
- 设置存储从 tauri-plugin-store 改为 core 层文件存储，调度器与平台层解耦

### 窗口与交互

- 修复 release 构建误连 devUrl（localhost:1420）的问题（补上 custom-protocol feature）
- 窗口宽高锁定 506×560，位置固定右下角（自动扣除任务栏），彻底消除弹出/切换页面时的重定位与闪切
- 修复 Windows 右下角锚点在 200% 缩放下垂直位置浮高的问题
- 修复 Windows 原生阴影导致的边缘黑边/宽度偏差
- macOS 面板改为跟随鼠标点击位置弹出
- 「查看详情」链接统一用系统浏览器打开，不再闪现空白窗口
- 添加供应商主选单页补回返回按钮

### 平台接入

- Codex 登录改为官方新版设备码流程（旧浏览器授权页已废弃），支持「导入本机 CLI 凭证」并自动续期
- Codex 用量详情链接修正为官方 `chatgpt.com/codex/settings/usage`
- 凭证存储改用随机主密钥 + 0600 权限，自动迁移旧数据

### 稳定性

- 调度器：各 provider 并发抓取、统一 HTTP 超时、失败保留旧数据并标记网络错误
- Windows CI 冒烟测试放行单实例插件隐藏窗口，增加构建产物上传
- 支持移除已添加的供应商

## v0.9.2（2026-08-02）

- 修复 Windows 单击托盘闪退/一闪而过，增加崩溃日志落盘
- 纯菜单栏架构，启动零窗口

## v0.9.1（2026-08-02）

- 修复 Windows 添加供应商空页面（窗口 label 路由）

## v0.9.0（2026-08-02）

- 首个跨平台菜单栏版本：OpenAI Codex / Platform、Kimi Code、Moonshot、DeepSeek、腾讯 TokenHub 额度监控
