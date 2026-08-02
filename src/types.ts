// 与 Rust 侧 providers/mod.rs 的统一数据模型严格对齐（serde 序列化后的 JSON 形态）。
// 字段名遵循 serde 默认 snake_case。任何 Rust 侧字段变更都必须同步到这里。

export type BillingMode = "Subscription" | "PayAsYouGo";

export type QuotaUnit =
  | "Percent"
  | "Requests"
  | "Tokens"
  | { Currency: string };

export type WindowPeriod =
  | "Hours5"
  | "Day"
  | "Week"
  | "Month"
  | { Custom: number };

export interface QuotaWindow {
  period: WindowPeriod;
  label: string;
  used: number | null;      // 百分比（0-100），无上限窗口为 null
  used_raw: number | null;  // 原始用量（金额 / token 数 / 请求数），用于数值文案
  limit: number | null;
  remaining: number | null;
  unit: QuotaUnit;
  reset_at: number | null;  // Unix 秒
}

export interface Balance {
  total: number;
  granted: number | null;
  topped_up: number | null;
  currency: string;
  available: boolean;
}

export type Fidelity = "Exact" | "Estimated" | "Partial";

export type HealthStatus =
  | "Ok"
  | "AuthExpired"
  | "Degraded"
  | "Exhausted"
  | "NetworkError";

export interface ProviderSnapshot {
  provider_id: string;
  display_name: string;
  plan_name: string | null;
  billing: BillingMode;
  balance: Balance | null;
  windows: QuotaWindow[];
  fidelity: Fidelity;
  status: HealthStatus;
  fetched_at: number; // Unix 秒
  last_error: string | null; // 最近一次抓取失败说明（成功为 null）
}

// ---------- 添加供应商向导的认证规格 ----------

export interface AuthField {
  key: string;
  label: string;
  placeholder: string;
  secret: boolean;
  required: boolean;
  options: [string, string][] | null;
}

export type AuthSpec =
  | { kind: "oauth"; authorize_url: string; token_url: string; client_id: string; scopes: string[]; pkce: boolean }
  | { kind: "api_key"; fields: AuthField[]; hint: string }
  | { kind: "cloud_secret"; fields: AuthField[] }
  | { kind: "hybrid"; primary: AuthSpec; fallback: AuthSpec };

export interface AddableProvider {
  id: string;
  display_name: string;
  auth_spec: AuthSpec;
}
