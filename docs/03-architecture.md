# 架构设计

---

## 技术选型

**Tauri 2 + Rust + TypeScript**

| 指标 | Tauri 2 | Electron |
|------|---------|----------|
| 安装包 | 10–15 MB | 100 MB+ |
| 常驻内存 | 30–50 MB | 150 MB+ |
| 凭证存储 | 系统 Keychain / Credential Manager | 需自行加密 |
| 托盘 API | 双端原生 | 双端原生 |
| 内嵌 WebView 注入 | `WebviewWindow` + init script | `BrowserWindow` + preload |

菜单栏应用是常年驻留的，内存是硬指标。加上要托管 8 组密钥（其中 OpenAI Admin Key 和腾讯 SecretKey 权限极大），能直接落到系统级安全存储是决定性优势。

代价：需要 Rust 工具链（约 1.5 GB）。Linux 需 WebKitGTK，但当前不在目标平台内。

---

## 统一数据模型

把 8 个平台的差异收敛到一套结构，UI 层不感知平台细节。

```rust
pub enum BillingMode {
    Subscription,   // 订阅：有套餐名 + 周期窗口
    PayAsYouGo,     // 按量：有账户余额 + 消耗统计
}

pub enum QuotaUnit {
    Percent,            // Codex 只给百分比
    Requests,           // 腾讯 Coding Plan 按请求次数
    Tokens,             // 腾讯 Token Plan
    Currency(String),   // "CNY" / "USD"
}

pub enum WindowPeriod {
    Hours5,     // Codex / Kimi Code / 腾讯 CP
    Day,
    Week,
    Month,
    Custom(i64),
}

pub struct QuotaWindow {
    pub period:    WindowPeriod,
    pub label:     String,          // "5 小时" / "本周" / "本月"
    pub used:      Option<f64>,
    pub limit:     Option<f64>,
    pub remaining: Option<f64>,
    pub unit:      QuotaUnit,
    pub reset_at:  Option<i64>,
}

pub struct Balance {
    pub total:     f64,
    pub granted:   Option<f64>,
    pub topped_up: Option<f64>,
    pub currency:  String,
    pub available: bool,
}

pub enum Fidelity {
    Exact,       // 官方接口直接返回
    Estimated,   // 余额差分推算
    Partial,     // 部分维度缺失
}

pub enum HealthStatus {
    Ok,
    AuthExpired,      // 需重新登录
    Degraded,         // 主路径失效，已降级
    Exhausted,        // 额度耗尽
    NetworkError,
}

pub struct ProviderSnapshot {
    pub provider_id:  String,
    pub display_name: String,
    pub plan_name:    Option<String>,   // "Plus" / "Pro 套餐" / "LEVEL_INTERMEDIATE"
    pub billing:      BillingMode,
    pub balance:      Option<Balance>,
    pub windows:      Vec<QuotaWindow>,
    pub fidelity:     Fidelity,
    pub status:       HealthStatus,
    pub fetched_at:   i64,
}
```

**关键设计**：`windows` 是数组而非固定字段。订阅制填 5h / 周 / 月，按量制填日 / 周 / 月消耗统计。同一套渲染逻辑通吃。

---

## Provider 抽象层

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn brand(&self) -> Brand;
    fn billing_mode(&self) -> BillingMode;

    /// 驱动「添加供应商」表单的动态渲染
    fn auth_spec(&self) -> AuthSpec;

    /// 探测本机是否已有可复用凭证（Codex CLI / kimi CLI）
    async fn detect_local(&self) -> Option<Credential>;

    async fn authenticate(&self, input: AuthInput) -> Result<Credential>;
    async fn refresh(&self, cred: &Credential) -> Result<Option<Credential>>;
    async fn fetch(&self, cred: &Credential) -> Result<ProviderSnapshot>;
}

pub enum AuthSpec {
    OAuth {
        authorize_url: &'static str,
        token_url:     &'static str,
        client_id:     &'static str,
        scopes:        &'static [&'static str],
        pkce:          bool,
    },
    ApiKey {
        fields: Vec<AuthField>,
        hint:   &'static str,
    },
    CloudSecret {
        fields: Vec<AuthField>,
    },
    Hybrid {
        primary:  Box<AuthSpec>,   // 逆向路径
        fallback: Box<AuthSpec>,   // 官方路径
    },
}

pub struct AuthField {
    pub key:         &'static str,
    pub label:       &'static str,
    pub placeholder: &'static str,
    pub secret:      bool,
    pub required:    bool,
    pub options:     Option<Vec<(&'static str, &'static str)>>,  // 下拉，如站点区域
}
```

`auth_spec()` 让「添加供应商」页面完全数据驱动——新增平台只需实现 trait，UI 一行不用改。

---

## 认证流程

### A. OAuth PKCE（Codex / Kimi Code）

```
1. 生成 code_verifier (43-128 随机字符) 与 code_challenge = BASE64URL(SHA256(verifier))
2. 起本地 HTTP server 监听随机高位端口
3. 系统默认浏览器打开授权 URL（不用内嵌 WebView —— 用户能看到真实域名和证书锁）
4. 用户在浏览器完成登录授权
5. 回调 http://localhost:<port>/auth/callback?code=xxx&state=yyy
6. 校验 state 防 CSRF，用 code + verifier 换 token
7. 关闭本地 server，凭证写入系统 Keychain
```

用系统浏览器而非内嵌 WebView 是刻意的：用户能确认自己是在 `auth.openai.com` 而不是钓鱼页，也能复用已有登录态和密码管理器。这是 opencodex 的做法，照抄。

### B. 本机凭证探测（一键导入）

添加供应商时先跑 `detect_local()`：

```
Codex      $CODEX_HOME/auth.json → ~/.config/codex/auth.json
           → ~/.codex/auth.json → macOS Keychain "Codex Auth"
Kimi Code  ~/.kimi/credentials/kimi-code.json
```

命中则在卡片上显示「检测到本机已登录，一键导入」，跳过整个 OAuth 流程。

### C. API Key / 云密钥对

表单由 `auth_spec()` 渲染。提交前做一次连通性校验（调一次余额接口），失败不落库，直接在表单内提示具体错误（401 = key 错，403 = 权限不足，站点选错则提示切换区域）。

### D. 混合（DeepSeek 增强路径）

内嵌 `WebviewWindow` 打开 `platform.deepseek.com`，注入 init script 监听登录完成，抓取会话凭证。用户可跳过，跳过则只用官方余额接口 + 差分。

---

## 凭证存储

```
macOS     Security.framework Keychain，service = "com.hangbits.tokenmeter"
Windows   Credential Manager (wincred)，target = "TokenMeter/<provider_id>"
```

统一走 `keyring` crate。**任何情况下不写明文到磁盘。**

非敏感配置（供应商启用状态、轮询间隔、UI 偏好）存 `SQLite`，与快照库同一个文件：

```
macOS    ~/Library/Application Support/TokenMeter/data.db
Windows  %APPDATA%\TokenMeter\data.db
```

---

## 调度器

```rust
pub struct Scheduler {
    providers: Vec<ProviderInstance>,
    tx: mpsc::Sender<SchedulerEvent>,
}
```

- 每个 provider 独立 tokio task，互不阻塞
- 单个 provider 失败不影响其他，指数退避重试（1s → 2s → 4s → … → 上限 5min）
- 连续失败 3 次标记 `HealthStatus`，菜单栏图标显示角标
- 监听系统休眠/唤醒事件（macOS `NSWorkspace` 通知，Windows `WM_POWERBROADCAST`），唤醒后立即补采
- 网络状态变化触发重连补采

轮询间隔见 `02-balance-diff.md` 的自适应策略。

---

## UI 规格

### 菜单栏 / 托盘

图标旁显示**最紧张的那个额度**（所有 provider 中剩余百分比最低的），而非全部堆砌。例如 `Codex 76%`。

点击展开面板，尺寸 380 × 自适应高度，最大 600。

### 供应商卡片

订阅制：

```
┌─────────────────────────────────────┐
│ ● OpenAI Codex            Plus  ⟳   │
│                                     │
│ 5 小时   ████████░░░░░░░░░░  6%     │
│                        4h 23m 后重置 │
│ 7 天     ████████████░░░░░░  24%    │
│                          3天后重置   │
│                                     │
│ Credits  $5.39                      │
└─────────────────────────────────────┘
```

按量制：

```
┌─────────────────────────────────────┐
│ ○ DeepSeek                      ⟳   │
│                                     │
│ 账户余额          ¥110.00           │
│   赠送 ¥10.00 · 充值 ¥100.00        │
│                                     │
│ 今日   ¥2.34      1.2M tokens       │
│ 本周   ¥15.80     8.4M tokens       │
│ 本月   ¥62.10    33.1M tokens       │
└─────────────────────────────────────┘
```

左上角圆点即 `Fidelity` 指示：实心 = 精确，空心 = 推算。悬停显示数据来源与最后成功时间。

### 添加供应商

两步式，借鉴 opencodex：

**Step 1 选择平台** — 按品牌分组的网格，每个品牌下列出其产品线：

```
OpenAI          Kimi              DeepSeek        腾讯云
├ Codex 订阅    ├ Kimi Code       └ DeepSeek      ├ Coding Plan
└ API Platform  └ Moonshot API                    ├ Token Plan
                                                  └ TokenHub 按量
```

卡片右下角标注认证方式徽章（`浏览器授权` / `API Key` / `云密钥`），让用户提前知道要准备什么。

**Step 2 认证** — 根据 `auth_spec()` 动态渲染：

- OAuth → 一个大按钮「在浏览器中登录」+ 检测到本机凭证时的「一键导入」
- API Key → 表单 + 获取密钥的跳转链接
- 云密钥对 → 两个字段 + 权限说明

### 主题

跟随系统 light / dark / auto 三态切换，与 macOS 菜单栏和 Windows 任务栏原生外观一致。切换无过渡闪烁。

---

## 项目结构

```
TokenMeter/
├── src/                        # 前端 TS
│   ├── components/
│   │   ├── ProviderCard.tsx
│   │   ├── QuotaBar.tsx
│   │   └── AddProviderWizard/
│   ├── stores/
│   └── main.tsx
├── src-tauri/
│   ├── src/
│   │   ├── main.rs
│   │   ├── tray.rs             # 菜单栏 / 托盘
│   │   ├── scheduler.rs
│   │   ├── store/
│   │   │   ├── keychain.rs
│   │   │   └── db.rs
│   │   ├── diff.rs             # 余额差分
│   │   └── providers/
│   │       ├── mod.rs          # trait 定义
│   │       ├── codex.rs
│   │       ├── openai_platform.rs
│   │       ├── kimi_code.rs
│   │       ├── moonshot.rs
│   │       ├── deepseek.rs
│   │       ├── tencent_coding_plan.rs
│   │       ├── tencent_token_plan.rs
│   │       └── tencent_tokenhub.rs
│   └── tauri.conf.json
└── docs/
```

---

## 里程碑

**M1 骨架** — Tauri 壳 + 托盘 + Provider trait + Keychain + SQLite

**M2 官方路径** — Moonshot / DeepSeek 余额、腾讯 Token Plan + TokenHub 按量、OpenAI Platform。全部走官方 API，风险最低，先跑通闭环。

**M3 OAuth 路径** — Codex + Kimi Code，含本机凭证探测。

**M4 差分引擎** — 快照库 + 差分算法 + 日/周/月聚合 + 可信度标注。

**M5 逆向增强** — DeepSeek 控制台内部接口 + 自动降级；腾讯 Coding Plan 额度查询（三条候选路径实测）。

**M6 打磨** — 主题、动效、告警阈值、开机自启、Windows 签名与分发。

M5 风险最高且随时可能失效，刻意排在最后。M2 完成时产品已经可用。
