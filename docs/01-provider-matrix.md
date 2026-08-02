# Provider 能力矩阵

> 调研日期：2026-08-02
> 所有端点均已核实来源，标注 ⚠️ 的为非公开接口，随时可能失效。

四个平台实际拆分为 **8 个独立供应商条目**，因为同一品牌下的订阅制与按量制走完全不同的凭证体系，数据结构也不通用。

---

## 一览表

| # | 条目 | 品牌 | 计费形态 | 认证方式 | 稳定性 |
|---|------|------|---------|---------|--------|
| 1 | OpenAI Codex | OpenAI | 订阅 | OAuth PKCE | ⚠️ 逆向 |
| 2 | OpenAI Platform | OpenAI | 按量 | Admin API Key | ✅ 官方 |
| 3 | Kimi Code | Kimi | 订阅 | OAuth | ⚠️ 逆向 |
| 4 | Moonshot API | Moonshot | 按量 | API Key | ✅ 官方 |
| 5 | DeepSeek | DeepSeek | 按量 | API Key + ⚠️Cookie | 混合 |
| 6 | 腾讯 Coding Plan | Tencent | 订阅（请求次数） | ⚠️ 待定 | ⚠️ 逆向 |
| 7 | 腾讯 Token Plan | Tencent | 订阅（Token 数） | SecretId/Key | ✅ 官方 |
| 8 | 腾讯 TokenHub 按量 | Tencent | 按量 | SecretId/Key | ✅ 官方 |

---

## 1. OpenAI Codex（ChatGPT 订阅侧）

**认证** — OAuth 2.0 + PKCE

```
授权端点   https://auth.openai.com/oauth/authorize
令牌端点   https://auth.openai.com/oauth/token
client_id  app_EMoamEEZ73f0CkXaXp7hrann
回调       http://localhost:<随机端口>/auth/callback
```

复用 Codex CLI 的 client_id。若本机已装 Codex CLI，可直接读取现成凭证跳过登录：

```
$CODEX_HOME/auth.json
~/.config/codex/auth.json
~/.codex/auth.json
macOS Keychain  service = "Codex Auth"
```

凭证结构：

```jsonc
{
  "OPENAI_API_KEY": null,
  "tokens": {
    "access_token": "",
    "refresh_token": "",
    "id_token": "",
    "account_id": ""      // 作为 ChatGPT-Account-Id 请求头
  },
  "last_refresh": "2026-01-28T08:05:37Z"
}
```

**额度端点**

```
GET https://chatgpt.com/backend-api/wham/usage
Authorization: Bearer <access_token>
ChatGPT-Account-Id: <account_id>
Accept: application/json
```

```jsonc
{
  "plan_type": "plus",
  "rate_limit": {
    "primary_window":   { "used_percent": 6,  "reset_at": 1738300000, "limit_window_seconds": 18000 },
    "secondary_window": { "used_percent": 24, "reset_at": 1738900000, "limit_window_seconds": 604800 }
  },
  "code_review_rate_limit": {
    "primary_window": { "used_percent": 0, "reset_at": 1738900000, "limit_window_seconds": 604800 }
  },
  "credits": { "has_credits": true, "unlimited": false, "balance": 5.39 }
}
```

映射：`primary_window` → 5 小时窗口，`secondary_window` → 7 天窗口。两个窗口同时生效，任一打满即限流。

**刷新** — `last_refresh` 超过 8 天或遇 401/403 时：

```
POST https://auth.openai.com/oauth/token
Content-Type: application/x-www-form-urlencoded

grant_type=refresh_token&client_id=app_EMoamEEZ73f0CkXaXp7hrann&refresh_token=<rt>
```

**注意** — 只给百分比，不给剩余条数。UI 应显示「已用 24%」而非伪造绝对值。

---

## 2. OpenAI Platform（API 按量侧）

**认证** — Admin API Key（`sk-admin-…`，需组织 Owner 权限创建，与普通 `sk-proj-` key 不同）

**端点**

```
GET https://api.openai.com/v1/organization/costs
      ?start_time=<unix>&bucket_width=1d&limit=31
GET https://api.openai.com/v1/organization/usage/completions
      ?start_time=<unix>&bucket_width=1d&group_by=model
```

返回按时间桶聚合的花费（USD）与 token 数（input/output/cached）。日/周/月消耗直接由时间桶累加得出，**无需差分推算**。

**注意** — Admin API Key 权限很大，必须存入系统 Keychain，且在 UI 上明确提示权限范围。

---

## 3. Kimi Code（coding plan 订阅侧）

**认证** — OAuth 2.0

```
认证服务   https://auth.kimi.com
client_id  17e5f671-d194-4dfb-9706-5516cb48c098
scope      kimi-code
本地凭证   ~/.kimi/credentials/kimi-code.json
```

```jsonc
{
  "access_token": "",
  "refresh_token": "",
  "expires_at": 1769861835.261056,
  "scope": "kimi-code",
  "token_type": "Bearer"
}
```

**额度端点**

```
GET https://api.kimi.com/coding/v1/usages
Authorization: Bearer <access_token>
```

```jsonc
// ✅ 2026-08-02 真实账号实测（设备码 OAuth 打 api.kimi.com/coding/v1/usages，HTTP 200）
{
  "user": {
    "userId": "cpomur...kffg",
    "region": "REGION_CN",                       // ← 区域字段真实存在
    "membership": { "level": "LEVEL_ADVANCED" },  // ← 会员档位真实存在
    "businessId": ""
  },
  "usage":  { "limit": "100", "used": "20", "remaining": "80", "resetTime": "2026-08-06T11:50:44Z" },  // 周额度
  "limits": [ { "window": { "duration": 300, "timeUnit": "TIME_UNIT_MINUTE" },
                "detail": { "limit": "100", "used": "42", "remaining": "58", "resetTime": "..." } } ],    // 5 小时窗
  "parallel": { "limit": "30" },                 // 并发上限
  "totalQuota": {},                              // ← 条件性字段：该账号为空对象
  "authentication": { "method": "METHOD_ACCESS_TOKEN", "scope": "FEATURE_CODING" },
  "subType": "TYPE_PURCHASE",
  "boosterWallet": {
    "balance": { "amount": "10000000000", "unit": "UNIT_CURRENCY", "type": "BOOSTER" },
    "monthlyChargeLimitEnabled": true,
    "monthlyChargeLimit": { "currency": "CNY", "priceInCents": "10000" },  // 月限额 ¥100
    "monthlyUsed":       { "currency": "CNY", "priceInCents": "10000" }    // 本月已用 ¥100（已用满）
  },
  "domain": "DOMAIN_NEXUS"
}
```
> ⚠️ **所有数值字段均为字符串**（`"100"`、`"10000000000"`），Rust 侧必须 string→number 解析。字段名大小写有版本漂移（官方 CLI parser 同时兼容 `used`/`remaining`、`resetAt`/`reset_at`）。

**会员档位与区域（2026-08-02 实测修正，推翻此前"API 不返回"的错误结论）：**
- `user.membership.level` **真实返回**，实测锚点已有两个：`LEVEL_INTERMEDIATE`（旧样本）、`LEVEL_ADVANCED`（本次实测）。→ 映射表：`LEVEL_BASIC→Andante` / `LEVEL_INTERMEDIATE→Moderato` / `LEVEL_ADVANCED→Allegretto` / `LEVEL_PRO→Allegro` / 第 5 档 slug+名待校准。fallback `Kimi 会员 L{level}`。
- `user.region` **真实返回**（`REGION_CN`）。level 与 region 是两个独立字段——佐证用户的"统一 level 体系 + 区域屏蔽可购档位"理论：level 全球一致，区域单独标注，故映射表保持单表，region 可用于校准档位显示名或标注账号区域。
- 官方 CLI `managed-usage.ts` / opencodex 的 parser 不读这两个字段，**但那是它们的取舍，不代表 API 不返回**——本轮实测直接证实。（教训：parser 不解析 ≠ 服务端不下发。）

**额度层数 = 2 + 条件层，不写死三层：**
- ① 5 小时滚动窗（`limits[]`，duration=300min）② 本周（`usage`，Weekly，带 resetTime）——这两层恒定存在。
- ③ `totalQuota` **条件性存在**（实测账号为空对象 `{}`）——有则渲染"总订阅额度"层，无则不渲染，前端按字段存在性驱动。
- ④ `boosterWallet` = Extra Usage 货币钱包（proto Money 风格字符串分），**独立余额型数据**：`monthlyUsed / monthlyChargeLimit` 即"本月 Extra Usage 用量/限额"，实测账号已用满（¥100/¥100）。应作为卡片独立行展示，不混入百分比额度条。
- 附加可展示元数据：`parallel.limit`（并发上限 30）、`subType`、`domain`。

**月限额 / 续费时间（2026-08-02 调研，方案定调 A + M5 可选增强）：**
- 用户在 `kimi.com/membership/subscription` 看到的"月限额总用量 + 自动续费/月重置时间"，**不在 coding usages 接口里**，走另一套 **`MembershipService`**（proto `kimi.gateway.membership.v2.membership`，Connect-RPC）。方法：`getSubscription`/`listSubscriptions`，字段含 `duration`、`nextSubscription*`（续费/下期时间）、`bonus`/`usingBonus`（月额度总量/已用）、`membershipLevel`、`capabilities[]`。
- **鉴权硬边界（opencodex 2026-07-18 实测 + kimi-web JS 包互证）**：该服务用 kimi.com **主站 web 会话**的 accessToken（`Authorization: Bearer`），**Kimi Code 的 OAuth token（scope=`FEATURE_CODING`）打它 → HTTP 401**。两套 token 互不通用（你的实测返回里 `authentication.scope=FEATURE_CODING` 即铁证）。
- **web-token 不能"无感自动"获得**：① App 内嵌 WebView 让用户主动登一次 kimi.com（可行但非无感、会话短命、风控灰色）② 读系统浏览器已登录 cookie（macOS Safari cookie 受完全磁盘访问保护 / Chrome 有 App-Bound Encryption，2024 后基本堵死）③ 复用官方 CLI 本地凭证（只有 coding scope，无 membership 权限）。**三条路都不体面。**
- **决策（用户已同意）**：M2/M3 只做 coding 数据（5h/周/Extra Usage 钱包 + 档位徽章）；月限额/续费时间列为 **M5 可选增强**——届时做"高级模式：内嵌 WebView 登录 kimi.com 拿完整订阅数据"的用户自愿开关，知情、可随时关。

**刷新** — 到期前 5 分钟主动刷新：

```
POST https://auth.kimi.com/api/oauth/token
client_id=17e5f671-d194-4dfb-9706-5516cb48c098&grant_type=refresh_token&refresh_token=<rt>
```

刷新被拒（401/403）时提示用户重新执行 `kimi login`。

---

## 4. Moonshot API（按量侧）

**认证** — API Key（`sk-…`）

**端点**

```
GET https://api.moonshot.cn/v1/users/me/balance     # 国内站，CNY
GET https://api.moonshot.ai/v1/users/me/balance     # 国际站，USD
Authorization: Bearer <MOONSHOT_API_KEY>
```

```jsonc
{
  "code": 0,
  "data": { "available_balance": 49.58894, "voucher_balance": 46.58893, "cash_balance": 3.00001 },
  "scode": "0x0",
  "status": true
}
```

**坑** — `platform.moonshot.cn`（国内）与 `platform.moonshot.ai`（国际）的 Key 完全独立，跨用返回 401。添加供应商时必须让用户选择站点区域。

**✅ 2026-08-02 实测修正：**
- 返回字段是 `data.available_balance / voucher_balance / cash_balance`（带 `_balance` 后缀 + 外层 `data/code/scode/status` 包装），实测账号余额为整数（50），有余额小数时才显示小数——Rust 仍用 f64。
- `voucher_balance` 独立 → 代金券与现金分开（代金券过期识别用）。
- **控制台已并入 kimi 平台**（2026-08-02 实测域名跳转关系）：国内 `platform.moonshot.cn` → `platform.kimi.com`、国际 `platform.moonshot.ai` → `platform.kimi.ai`。**API 端点仍用 moonshot 域名**（`api.moonshot.cn` / `api.moonshot.ai`，不跳转）；仅控制台走 kimi。详情页统一 `https://platform.moonshot.cn/console/account`（自动跳对区域，国际站用 .ai）。

**缺口** — 无用量接口，仅实时余额（本 App 已决定不做差分统计，见 MEMORY"零数据库"决策）。

---

## 5. DeepSeek

**认证 A（官方）** — API Key

```
GET https://api.deepseek.com/user/balance
Authorization: Bearer <DEEPSEEK_API_KEY>
```

```jsonc
{
  "is_available": true,
  "balance_infos": [
    { "currency": "CNY", "total_balance": "110.00", "granted_balance": "10.00", "topped_up_balance": "100.00" }
  ]
}
```

**认证 B（⚠️ 逆向，可选增强）** — 内嵌 WebView 登录 `platform.deepseek.com`，Hook 抓取会话凭证后调用控制台前端内部接口，可取得按模型 / 按日的 token 数、金额、缓存命中率。

**降级策略** — B 失效自动回落到 A + 余额差分，UI 标注数据来源与可信度。详见 `02-balance-diff.md`。

---

## 6. 腾讯 Coding Plan（订阅，请求次数计量）

**套餐规格**

| 套餐 | 价格 | 每 5 小时 | 每周 | 每订阅月 |
|------|------|----------|------|---------|
| Lite | 40 元/月 | ~1,200 次 | ~9,000 次 | ~18,000 次 |
| Pro | 200 元/月 | ~6,000 次 | ~45,000 次 | ~90,000 次 |

三级窗口天然对应「小时 / 周 / 月」三档展示，是四个平台里唯一原生具备完整三级窗口的。

**额度查询** — ⚠️ 官方未提供 API。三条候选路径，需实测确认：

- **C1** 逆向 Coding Plan 控制台页面接口（需登录态 Cookie）
- **C2** 调用 `api.lkeap.cloud.tencent.com/plan/v3` 时读取响应头的 `x-ratelimit-*` 系列字段
- **C3** 本地记账兜底

优先验证 C2 —— 成本最低且不需要额外登录态。若响应头不含配额信息，退到 C1。

**Base URL**

```
OpenAI 协议     https://api.lkeap.cloud.tencent.com/plan/v3
Anthropic 协议  https://api.lkeap.cloud.tencent.com/plan/anthropic
```

**合规提示** — 腾讯明令 Coding Plan「严禁 API 调用，仅限在编程工具中使用」。本 App 仅做**只读额度查询**，不产生模型推理调用，不触碰该限制。但 C2 方案依赖真实推理请求的响应头，因此**不主动发起探测请求**，只能被动记录——这一点需要在实现时重新评估，必要时放弃 C2。

---

## 7. 腾讯 Token Plan（订阅，Token 数计量）

分「通用 Token Plan」与「Hy Token Plan」两个独立套餐，共用同一 API Key，按 Model ID 自动路由扣减。

| 档位 | 通用 | Hy | 月度 Token |
|------|------|-----|-----------|
| Lite | 39 元 | 28 元 | 3,500 万 |
| Standard | 99 元 | 78 元 | 1 亿 |
| Pro | 299 元 | 238 元 | 3.2 亿 |
| Max | 599 元 | 468 元 | 6.5 亿 |

**🔴 2026-08-02 实测重大发现：个人版 ≠ 企业版，两套体系，且个人版无查用量 API。**

| 维度 | 个人版（你买的这种） | 企业版 |
|------|---------------------|--------|
| 凭证 | **专属 API Key**（`sk-tp-...`，套餐页生成） | CAM SecretId / SecretKey |
| Base URL | `tokenhub[-intl].tencentcloudmaas.com` | `tokenhub.tencentcloudapi.com` |
| 查用量 | ❌ **无 API**，仅控制台网页可见 | ✅ `DescribeTokenPlan(List)`（TC3 签名） |
| 推理 | ✅ OpenAI 兼容 `/v1/chat/completions` | ✅ |

**个人版实测（你的 `sk-tp-` Key，2026-08-02）：**
- 域名 `tokenhub[-intl].tencentcloudmaas.com` **只暴露 OpenAI 兼容推理 API**（`/v1/models`、`/v1/chat/completions`）。
- **所有查用量/余额/订阅端点均 404**：`/v1/usage`、`/v1/dashboard/billing/*`、`/v1/organization/usage`、`/plan/v3/usage|quota|credits|subscription` 全部不存在。
- 你的 Key 打 `/v1/models` 返回 401（signature 校验失败）——该 Key 专用于推理调用，可能绑定特定域名/签名，**非通用 REST Key**。
- **结论：个人版 Token Plan 的"剩余额度"无官方 API**，只能靠 ① 本地记账（App 记录每次调用 token 数，月上限−累计反推）或 ② 逆向控制台内部接口（M5，灰色，需腾讯云登录 cookie）。**M2 官方路径对个人版走不通。**

**企业版实测（CAM SecretId/Key，Region=`ap-guangzhou`，2026-08-02）：**
- TC3 签名 ✅、版本号 `2026-03-22` ✅、`X-TC-Region` 必填。
- `DescribeTokenPlanList` 打通，但**面向企业版**（`tp-ent-` 前缀 TeamId）。实测你的账号返回 `{"TokenPlanSet": [], "TotalCount": 0}`——因你是个人版，企业版列表自然为空。⚠️ Provider 逻辑须区分"接口报错" vs "无企业版套餐"（后者不告警）。
- CAM 权限：需 `tokenhub:DescribeTokenPlan*` 读权限（预设 `QcloudTokenHubReadOnlyAccess` 或自定义策略）。

**端点（企业版）**

```
POST https://tokenhub.tencentcloudapi.com
X-TC-Action: DescribeTokenPlanList   # 先拿列表（含主额度包详情）
X-TC-Version: 2026-03-22
X-TC-Region: ap-guangzhou            # 必填
```

返回：`TokenPlanSet[]`（每个套餐含名称/状态/主额度包余量）、`TotalCount`。再按 `TeamId` 调 `DescribeTokenPlan` 取详情（`Status`、`StopReason`：NORMAL / ISOLATED / FROZEN / EXHAUSTED / DESTROYED）。

**注意** — 额度不结转，套餐到期 API Key 立即失效。`StopReason = EXHAUSTED` 应在菜单栏红色告警。

---

## 8. 腾讯 TokenHub 按量

**认证** — 同上 SecretId / SecretKey

**端点**

```
POST https://tokenhub.tencentcloudapi.com
X-TC-Action: DescribeUsageRankList
X-TC-Version: 2026-03-22
```

参数 `Dimension` 取 `apikey` / `endpoint` / `model`，`MetricType=tokens`。

返回指标：`TotalToken` / `InputTotalToken` / `OutputTotalToken` / `CacheTotalToken`，含 `TotalStats`（整段聚合）与 `TopList`（逐时间点曲线）。

**账户余额**

```
POST https://billing.tencentcloudapi.com
X-TC-Action: DescribeAccountBalance
```

**评价** — 腾讯是四家里唯一官方同时提供「余额 + token 用量 + 时间曲线」的，数据完整度最高。

---

## 数据可得性总结

| 指标 | Codex | OpenAI API | Kimi Code | Moonshot | DeepSeek | 腾讯 CP | 腾讯 TP | 腾讯按量 |
|------|:-----:|:----------:|:---------:|:--------:|:--------:|:-------:|:-------:|:--------:|
| 套餐名称 | ✅ | — | ✅ | — | — | ⚠️ | ✅ | — |
| 账户余额 | ✅ credits | — | — | ✅ | ✅ | — | ✅ 额度包 | ✅ |
| 小时窗口 | ✅ 5h | — | ✅ 5h | — | — | ⚠️ 5h | — | — |
| 周窗口 | ✅ 7d | 桶聚合 | — | — | — | ⚠️ | — | 桶聚合 |
| 月窗口 | — | 桶聚合 | — | — | — | ⚠️ | ✅ | 桶聚合 |
| Token 数 | — | ✅ | — | ✖ | ⚠️ | — | ✅ | ✅ |
| 金额 | ✅ | ✅ | — | 差分 | 差分 | — | — | ✅ |

✅ 官方直接可得 ｜ ⚠️ 需逆向或待验证 ｜ ✖ 完全不可得 ｜ 差分 = 余额快照推算
