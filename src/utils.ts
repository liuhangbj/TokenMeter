// 工具函数 —— 5 段警示色、时间格式化与标准品牌样式

import type { CSSProperties } from "react";
import type { BrandStyle } from "./types";
import type { HealthStatus } from "./types";

/** 5 段用量等级（1-5）。hasCap=false 表示无上限（余额/成本型），返回 0 走品牌色。 */
export function usageLevel(pct: number | null, hasCap: boolean): number {
  if (!hasCap || pct === null) return 0;
  if (pct >= 100) return 5;
  if (pct >= 90) return 4;
  if (pct >= 80) return 3;
  if (pct >= 50) return 2;
  return 1;
}

export function levelClass(lv: number): string {
  return lv >= 1 && lv <= 5 ? `lv${lv}` : "brand";
}

/** 健康状态 → 状态点 class。AuthExpired/Exhausted 等映射到对应色。 */
export function statusDotClass(status: HealthStatus, maxLv: number): string {
  switch (status) {
    case "AuthExpired":
    case "NetworkError":
      return "stale";
    case "Exhausted":
      return "lv5";
    case "Degraded":
      return "lv3";
    case "Ok":
    default:
      return maxLv >= 1 ? `lv${maxLv}` : "lv1";
  }
}

/** Unix 秒 → "X 天/X 小时/X 分钟后重置" */
export function resetIn(resetAt: number | null): string {
  if (!resetAt) return "";
  const secs = resetAt - Math.floor(Date.now() / 1000);
  if (secs <= 0) return "即将重置";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d} 天后重置`;
  if (h > 0) return `${h} 小时后重置`;
  return `${m} 分钟后重置`;
}

/** Unix 秒 → "X 分钟前" */
export function updatedAgo(fetchedAt: number): string {
  const secs = Math.floor(Date.now() / 1000) - fetchedAt;
  if (secs < 60) return "刚刚";
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m} 分钟前`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h} 小时前`;
  return `${Math.floor(h / 24)} 天前`;
}

/** 金额格式化：按币种符号 + 千分位 + 最多 2 位小数 */
export function fmtMoney(amount: number, currency: string): string {
  const sym = currency === "CNY" ? "¥" : currency === "USD" ? "$" : currency + " ";
  const v = amount.toLocaleString("en-US", { maximumFractionDigits: 2 });
  return `${sym}${v}`;
}

/** Token / 请求数紧凑显示：1.25K、8M、4.2B；原始值仍用于计算。 */
export function fmtTokens(n: number): string {
  if (!Number.isFinite(n)) return "—";
  const absolute = Math.abs(n);
  if (absolute < 1_000) return Math.round(n).toLocaleString("en-US");

  const units = [
    { threshold: 1_000_000_000_000, suffix: "T" },
    { threshold: 1_000_000_000, suffix: "B" },
    { threshold: 1_000_000, suffix: "M" },
    { threshold: 1_000, suffix: "K" },
  ];
  const unit = units.find(({ threshold }) => absolute >= threshold)!;
  const scaled = n / unit.threshold;
  const scaledAbsolute = Math.abs(scaled);
  const maximumFractionDigits = scaledAbsolute >= 100 ? 0 : scaledAbsolute >= 10 ? 1 : 2;
  return `${scaled.toLocaleString("en-US", { maximumFractionDigits })}${unit.suffix}`;
}

/** 后端品牌配置 → 支持明暗模式的 CSS 变量；新供应商无需新增 CSS 选择器。 */
export function brandStyleVars(brand: BrandStyle): CSSProperties {
  return {
    "--brand-accent-light": brand.accent_light,
    "--brand-accent-alt-light": brand.accent_alt_light,
    "--brand-accent-dark": brand.accent_dark,
    "--brand-accent-alt-dark": brand.accent_alt_dark,
  } as CSSProperties;
}
