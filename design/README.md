# TokenMeter · 下拉面板设计规格

> 无主窗口，仅点菜单栏图标弹下拉面板。按各平台「能拿到的数据」分 4 种标准卡片模板，统一设计语言。
> 高保真可交互预览见 `popover-preview.html`（点右上角图标唤出，支持明暗主题）。

## 一、统一设计语言（所有模板共用）

| 维度 | 规范 |
|------|------|
| 面板 | 宽 380px，毛玻璃 `blur(34px)` + 饱和，圆角 22px，柔和投影，从图标下方弹出 |
| 卡片壳 | 圆角 18px，左侧 3px 品牌渐变条，hover 上浮 2px，统一内边距 |
| 品牌色 | 平台主色系（官方核实）：OpenAI 绿 `#10a37f`、Kimi/Moonshot 黑 `#000`、DeepSeek 蓝 `#4d6bfe`、腾讯蓝 `#0052d9`；深色模式下黑色品牌自动提亮 |
| 状态点 | 用量提醒 5 段（见「配色系统」节）：浅绿<50% / 深绿50–80% / 橙80–90% / 红90–100% / 深红用完；与进度条同色 |
| 可信度 | 实心点=官方接口精确；空心点=余额差分推算；琥珀实心=部分缺失 |
| 数字 | `tabular-nums` 等宽对齐，hero 数字 30px 粗体 |
| 主题 | 明/暗双套 CSS 变量，切换无闪烁 |
| 微交互 | 进度条入场动画 + 微光 shimmer；刷新按钮旋转；卡片 hover 上浮 |

## 一之一、配色系统（平台主色 + 用量警示）

### 平台主色（品牌条 / 圆点 / 套餐徽章来源）
> 来源：各平台官方品牌规范（OpenAI 双色黑+绿、月之暗面黑白、DeepSeek Wikidata sRGB、腾讯蓝 Pantone 2728C）

| 品牌 | 条目 | 主色 | 说明 |
|------|------|------|------|
| OpenAI | Codex、Platform | 绿 `#10a37f` | 用签名绿（黑是另一官方色），与 Kimi 黑区分 |
| Kimi | Kimi Code | 黑 `#000000` | 月之暗面官方仅黑白 |
| Moonshot | Moonshot API | 黑 `#000000` | 同属月之暗面，与 Kimi 同色（按产品名区分） |
| DeepSeek | DeepSeek | 蓝 `#4d6bfe` | 电光蓝 |
| 腾讯 | Coding Plan / Token Plan / TokenHub | 蓝 `#0052d9` | 腾讯蓝 |

深色模式下 `--accent` 对 `kimi`/`moonshot` 提亮为 `#5a5a60`，否则黑色条在深色面板不可见。

### 用量提醒 5 段（进度条填充 + 状态点同色，满=危险）
| 区间 | 颜色 | 色值 |
|------|------|------|
| &lt;50% | 浅绿 | `#7ed79b` |
| 50–80% | 深绿 | `#2ba471` |
| 80–90% | 橙 | `#e37318` |
| 90–100% | 红 | `#d54941` |
| 用完（≥100% 或耗尽） | 深红 | `#9e2b25` |

> 该阈值仅作用于「有百分比上限」的模板（额度型 `quota`、套餐包型 `package`）。余额消耗型 `balance` / 成本型 `cost` 无百分比上限，进度条/状态点沿用品牌色，不套用此 ramp。

## 二、4 种标准模板

### 模板 1 · 订阅额度型 `quota`
**适用**：OpenAI Codex、Kimi Code、腾讯 Coding Plan
**数据来源**：`BillingMode::Subscription` + `windows[]`（含 `used%` 与 `reset_at`）
**布局**：套餐徽章 + 每个周期一条进度条（周期标签 / 重置倒计时 / 用量%）
**注意（2026-08-02 Kimi 实测修正）**：Kimi Code 的 `/usages` 真实返回——
- **套餐徽章**：`user.membership.level` 实测返回（锚点 `LEVEL_INTERMEDIATE`/`LEVEL_ADVANCED`），按单表映射显示档名（Andante/Moderato/Allegretto/Allegro/Lv5）；`user.region`（`REGION_CN`）独立返回，佐证"统一 level + 区域屏蔽"理论。
- **额度层数 = 2 + 条件层，不写死三层**：① 5 小时滚动窗（`limits[]`）② 本周（`usage`）恒定存在；③ `totalQuota` 条件性存在（实测账号为空 `{}`）——**前端按字段存在性驱动渲染**，有则显示"总订阅额度"层，无则不渲染。
- **boosterWallet 独立行**：Kimi Extra Usage 货币钱包（proto Money 字符串分），`monthlyUsed/monthlyChargeLimit` 渲染为卡片内独立"Extra Usage · 本月"行，不混入百分比额度条。

### 模板 2 · 余额消耗型 `balance`
**适用**：DeepSeek、Moonshot、腾讯 TokenHub 按量
**数据来源**：`balance` + `windows[]`（token 数 + 金额）
**布局**：余额 hero 大数字 + 日/周/月消耗明细（金额优先、token 次要）
**可信度**：DeepSeek / Moonshot 无官方用量接口 → 消耗来自差分引擎，打「推算」空心点标签；腾讯 TokenHub 为官方精确

### 模板 3 · 成本聚合型 `cost`
**适用**：OpenAI Platform（管理后台 costs API）
**数据来源**：`windows[]` 为按成本的时间桶聚合，无 `balance`
**布局**：本月花费 hero + 周/日花费明细（纯金额，无余额概念）

### 模板 4 · 套餐包型 `package`
**适用**：腾讯 Token Plan
**数据来源**：`plan_name` + 额度包（剩余 / 总额）+ `status`（生效中/已暂停）+ 停用原因
**布局**：状态药丸 + **比例条**（剩余% + 已用进度，语义与额度型一致：满=危险）+ `剩余 / 总额` 大数字 + 备注
**比例计算**：`ratio = 剩余 / 总额`；进度条填充 `已用 = 100 - ratio`，按用量提醒 5 段着色（见「配色系统」节：<50% 浅绿 / 50–80% 深绿 / 80–90% 橙 / 90–100% 红 / 用完深红）。若真实 API 仅返回剩余、无总额，则用套餐容量常量推算（如企业版 5.00M）

## 三、与 Rust 数据模型的映射

前端卡片字段 **100% 来自 `ProviderSnapshot`**，不做平台特例硬编码：

```
ProviderSnapshot
├─ display_name / plan_name      → 卡片名 + 套餐徽章
├─ billing: BillingMode          → 选 quota / balance / cost / package 模板
├─ windows: Vec<QuotaWindow>     → 进度条 or 消耗明细
│   ├─ period/label/used/limit    → 周期标签 + 用量%
│   └─ unit (Percent/Count/Token/Currency) → 显示单位
├─ balance: Option<Balance>       → hero 余额
├─ fidelity: Fidelity             → 实心/空心/琥珀 可信度点
└─ status: HealthStatus           → 状态点颜色
```

> 模板选择建议在 Rust 侧加一个展示提示字段（如 `card_kind`），或在 TS 侧按 `billing` + 是否含 `balance` 推导。腾讯 Token Plan 因无百分比额度、仅有「包剩余」，归为 `package` 特例。

## 四、真实工程落地（React / Tauri）

面板是 Tauri **无装饰窗口**（`decorations:false`），定位到托盘图标下方（用 `tauri-plugin-positioner` 或手动算坐标）。结构建议：

```
src/
├─ theme.css                 # 本设计系统的 CSS 变量 + 卡片样式
├─ components/
│  ├─ Popover.tsx            # 窗口壳：头部(刷新/主题) + 卡片列表 + 添加按钮
│  ├─ cards/
│  │  ├─ QuotaCard.tsx       # 模板 1
│  │  ├─ BalanceCard.tsx     # 模板 2
│  │  ├─ CostCard.tsx        # 模板 3
│  │  └─ PackageCard.tsx     # 模板 4
│  └─ CardShell.tsx          # 统一壳（品牌条/头部/底状态行）
└─ lib/useSnapshots.ts       # 调 Rust command get_snapshots() → ProviderSnapshot[]
```

每个 `.tsx` 直接对应预览里的 `tplXxx()` 渲染函数，可 1:1 移植。

## 五、下一步

- 批准设计 → 我把预览移植为 React 组件 + 接 Rust `get_snapshots()` 命令（M6 前端）
- 或先迭代模板视觉（间距/配色/新增字段）
- 「添加供应商」向导作为独立窗口（M6 另一块），不在主下拉内
