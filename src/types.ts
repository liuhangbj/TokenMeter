// 与 Rust 侧 providers/presentation.rs 的标准卡片契约严格对齐。
// 字段名遵循 serde 默认 snake_case。任何 Rust 侧字段变更都必须同步到这里。

export type QuotaUnit =
  | "Percent"
  | "Requests"
  | "Tokens"
  | { Currency: string };

export interface BrandStyle {
  key: string;
  accent_light: string;
  accent_alt_light: string;
  accent_dark: string;
  accent_alt_dark: string;
}

export interface QuotaCardItem {
  kind: "quota";
  label: string;
  used_percent: number | null;
  used: number | null;
  limit: number | null;
  unit: QuotaUnit;
  reset_at: number | null;
}

export interface BalanceCardItem {
  kind: "balance";
  label: string;
  value: number | null;
  unit: QuotaUnit;
}

export type CardItem = QuotaCardItem | BalanceCardItem;

export type HealthStatus =
  | "Ok"
  | "AuthExpired"
  | "Degraded"
  | "Exhausted"
  | "NetworkError";

export interface AccountCardModel {
  account_id: string;
  account_label: string | null;
  provider: {
    id: string;
    name: string;
    brand: BrandStyle;
    detail_url: string | null;
  };
  plan: {
    name: string;
    tier: number | null;
  } | null;
  primary: {
    label: string;
    value: number | null;
    unit: QuotaUnit | null;
    health_used_percent: number | null;
  };
  items: CardItem[];
  status: HealthStatus;
  fetched_at: number;
  last_error: string | null;
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

export interface AddableVendor {
  id: string;
  display_name: string;
  brand: BrandStyle;
}

export interface AddableProvider {
  id: string;
  product_name: string;
  description: string;
  account_type: "plan" | "api";
  vendor: AddableVendor;
  brand: BrandStyle;
  auth_spec: AuthSpec;
  supports_local_import: boolean;
}

export type AppTheme = "classic" | "parchment" | "cyberpunk";
export type AppAppearance = "system" | "light" | "dark";
