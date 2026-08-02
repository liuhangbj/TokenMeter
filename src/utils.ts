// 工具函数 —— 5 段警示色、时间格式化、品牌/状态映射

import type { HealthStatus, Fidelity } from "./types";

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

/** 可信度点 */
export function fidMeta(f: Fidelity): [string, string] {
  if (f === "Exact") return ["exact", "官方接口"];
  if (f === "Estimated") return ["est", "推算/非官方来源"];
  return ["partial", "部分维度缺失"];
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

/** token 数格式化：千分位整数 */
export function fmtTokens(n: number): string {
  return Math.round(n).toLocaleString("en-US");
}

/** provider_id → 品牌 data-attribute（决定品牌色） */
export function brandOf(providerId: string): string {
  if (providerId.startsWith("codex") || providerId.startsWith("openai")) return "openai";
  if (providerId.startsWith("kimi")) return "kimi";
  if (providerId.startsWith("moonshot")) return "moonshot";
  if (providerId.startsWith("deepseek")) return "deepseek";
  if (providerId.startsWith("tencent")) return "tencent";
  return "openai";
}

/** 官方控制台链接（"查看详情↗"跳转） */
export function consoleUrl(providerId: string): string | null {
  switch (providerId) {
    case "codex": return "https://chatgpt.com/codex";
    case "openai_platform": return "https://platform.openai.com/usage";
    case "kimi_code": return "https://www.kimi.com/membership/subscription";
    // 2026-08-02 实测：Moonshot 控制台已并入 kimi 平台，moonshot 域名自动跳转对应区域
    // 国内 platform.moonshot.cn→platform.kimi.com、国际 platform.moonshot.ai→platform.kimi.ai。
    // 用 moonshot 官方域名 /console/account（自动跳对区域）；当前默认国内站。
    case "moonshot": return "https://platform.moonshot.cn/console/account";
    case "deepseek": return "https://platform.deepseek.com/usage";
    case "tencent_tokenhub": return "https://console.cloud.tencent.com/tokenhub";
    case "tencent_token_plan": return "https://console.cloud.tencent.com/tokenhub/token-plan";
    case "tencent_coding_plan": return "https://cloud.tencent.com/product/codingplan";
    default: return null;
  }
}
