# Provider 卡片标准契约

TokenMeter 的前端只消费 `AccountCardModel`，不读取各供应商原始字段，也不按
`provider_id` 分支。新增供应商时，UI 卡片由统一契约自动生成。

## 数据流

```text
供应商原始 API
  → Provider::fetch（认证、请求、原始字段解析）
  → ProviderSnapshot（平台无关领域数据）
  → Provider::card_config / plan_tier（展示语义映射）
  → AccountCardModel（稳定 UI 契约）
  → ProviderCard（唯一卡片组件）
```

## 前端支持的语义

- `primary`：卡片主值，支持百分比、金额、Token 和请求数。
- `quota`：有上限的额度，显示使用比例、原始用量、进度条与重置时间。
- `balance`：余额或无上限统计，只显示数值，不显示进度条。
- `plan.tier`：按大致月价统一为 0–5 档，未知套餐使用中性色。

主值对应的额度仍必须保留在 `items` 中，保证“主值 + 同一档进度条”同时存在。

## 新增供应商清单

1. 在独立 Provider 中完成认证、请求和原始响应解析。
2. 将返回字段映射为 `ProviderSnapshot` 的 `balance` / `windows`。
3. 通过 `card_config()` 声明主值、余额和特殊货币窗口的语义。
4. 通过 `plan_tier()` 映射标准套餐名称与 0–5 价格档位。
5. 在 `Brand::style()` 配置明暗模式品牌色；未知图标会自动使用名称首字。
6. 增加至少一个契约测试，验证主值、明细顺序、单位和套餐档位。

## 2026-08-12 新接入映射

| Provider | 认证 | 主值 | 明细 |
|---|---|---|---|
| Gemini Code Assist | Google OAuth / Gemini CLI 凭证 | 最长周期模型配额余量 | 全部模型桶、重置时间、AI Credits |
| 火山引擎 | 费用中心 AK/SK | `AvailableBalance` | `CashBalance`、`CreditLimit`、`FreezeAmount`、`ArrearsBalance` |
| SiliconFlow | 国内/国际站 API Key | `totalBalance` | `chargeBalance`、`balance` |

Gemini AI Studio 的普通 API Key 当前没有可用的账户余额/套餐额度接口，因此不创建
无法满足主值契约的空壳入口。火山方舟推理 API Key 与费用中心 AK/SK 也不能混用。

正常情况下不需要修改 `src/ProviderCard.tsx`、`src/App.tsx` 或新增 CSS 卡片模板。
