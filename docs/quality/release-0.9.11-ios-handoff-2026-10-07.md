# v0.9.11 桌面候选与 iOS 交接记录

> 日期：2026-10-07（Asia/Shanghai）。本文件记录候选范围与证据，不代表已经
> 提交、推送、打 tag、部署或发布。iOS 实施暂缓，桌面候选审验完成后再启动。

## 目标与边界

- 桌面 v0.9.11 纳入供应商映射/降级、Codex Pro 100/200/500、Codex 独立
  额度桶、通用套餐色阶修正和 macOS 27 菜栏点击兼容补丁。
- Windows 悬浮球首次启用修复没有 Windows 实机证据，本候选不纳入，也不在
  发布说明中宣称已修复。
- 日用 App、真实凭证、账号昵称/顺序/设置均不改动；不调用付费 API。
- 未获经理审验放行前不提交、推送、打 tag、部署或发布。

## 已确认问题与修复链

日用唯一进程 PID 31744 来自
`/Users/hangbits/Applications/TokenMeter Dev.app/Contents/MacOS/tokenmeter`，
版本 0.9.10，二进制 SHA-256 为
`30756ba70f43396d6abe7fbeddb46cb53b48e249d28762ce4e5fd77ca0aee976`。
该身份与历史 macOS 27 托盘补丁包一致，不含本轮 provider 修改。

基线源码对原始 `plan_type=pro` 只显示 `Pro`，`plan_tier()` 无匹配，最终进入
`tier-neutral`；这是当前 $200 套餐徽章颜色错误的已确认代码与安装差异，但不
声称读取过用户真实 API 原始响应。候选链路改为：

| 原始机器值 | 展示名 | 档位 | 主题语义 |
| --- | --- | --- | --- |
| `prolite` / 旧 `pro_5x` | Pro 100 | 4 | 橙色 |
| `pro` / 旧 `pro_20x` | Pro 200 | 5 | 玫红/红紫 |
| `promax` | Pro 500 | 5 | 现有最高价格档 |

新增 500 不改变 200 的既有价格语义。前端通用映射同时将数值 tier 0 映射到
既有 `tier-free`，修复此前 `tier-0` 没有 CSS 规则的问题；没有 provider 分支。

## Pro 500 与额外额度证据边界

- OpenAI Codex 固定源：`openai/codex` 提交
  `e95abcdf4939f37f11f00f984efdbbf8b088346e`。
- 官方协议将 `promax` 映射为 ProMax，订阅展示层对应 Pro 500；公开 schema
  含 `additional_rate_limits`，每个独立桶可有 primary/secondary window。
- 候选解析全部独立额度桶并保留名称、周期、用量和重置时间；这些窗口使用
  Custom 周期，只作为明细，不替换普通最长主窗口，也不改变主健康状态。
- Credit 仍为补充余额；明确 0 可见，缺失余额不虚构为 0。
- 合成 fixture 覆盖 Pro 500、普通 5h/7d、独立 30m/1d、Credit 与未知字段。
  没有真实 Pro 500 响应，因此结论仅为代码/schema/合成契约覆盖，真实账户待样本。

## 版本与发布元数据

候选版本统一为 0.9.11：`package.json`、`package-lock.json`、
`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`。
releaseBody 只描述本候选实际内容，不包含 Windows 悬浮球修复或 iOS 功能。

## iOS 本机边界

只读探测结果：Apple Silicon (`arm64`)，`xcode-select -p` 为
`/Library/Developer/CommandLineTools`；本机没有 `simctl`，也没有 iphoneos SDK。
按用户要求不安装 Xcode、不切换全局 developer directory。iOS 编译、模拟器
运行与截图必须在后续获准的远端 macOS/Xcode 环境完成；真机签名、后台/锁屏、
Widget 功耗及 StoreKit 仍需独立真机证据，不能由云编译替代。

## 候选冻结时补录

阶段自测完成后在此补录 base/head、patch SHA-256、文件清单、候选 App/二进制
身份、测试统计和完整日志路径。独立审核需分别给出技术结论与真实使用结论。

## 主开发阶段自测结果

- `cargo test --locked --no-fail-fast`：69/69 通过，0 失败、0 跳过。
- `npm run build`：TypeScript 与 Vite production build 通过。
- `cargo fmt --check`、`git diff --check`：通过。
- `cargo build --release --locked --features custom-protocol`：通过。
- `npm audit --omit=dev --audit-level=high`：生产依赖 0 漏洞；`npm ci` 的 1 个
  high 属开发依赖，未在本阶段自动升级依赖。
- Tauri `.app` 在临时覆盖 `createUpdaterArtifacts=false`、`--no-sign` 下打包通过；
  正式配置仍保持 updater 签名。未读取私钥，随后只对 QA 副本做 ad-hoc 签名。
- QA App：`/tmp/tokenmeter-desktop-0.9.11-evidence/qa.6EbvEp/`
  `TokenMeter-v0.9.11-QA.app`；bundle id `com.hangbits.tokenmeter`，版本 0.9.11，
  arm64，严格签名校验通过；签名后二进制 SHA-256
  `106759bc5b74cbab59abf0669bb48b7928941759ef790914063a1b84b2a5af45`。
- 六主题/外观：380×520 浏览器实测 Classic/Parchment/Cyber × light/dark；
  Free=`tier-free`，Pro100=`tier-4`，Pro200/500=`tier-5`，未知=`tier-neutral`。
  计算色证据在 `/tmp/tokenmeter-desktop-0.9.11-evidence/theme-computed-colors.json`。
- XHS 六图均为 1242×1656；`xhs.html` 23/23 素材存在；六图 OCR 未命中
  token、密钥或邮箱，唯一 `API Key` 命中是功能说明文字。
- README 六个本地图片引用均存在；四处版本及 Cargo.lock 已同步 0.9.11。

未执行/受阻：为保护正在运行的 0.9.10 日用 App 及真实数据，未启动同 bundle id
的 QA App、未刷新真实 Codex 账户；没有真实 Pro500 响应，Windows/Intel Mac、
完整原生托盘回归和发布 CI 也未执行。这些不记 PASS，留给冻结候选独立审验和
获批后的正式 CI/安装验证。
