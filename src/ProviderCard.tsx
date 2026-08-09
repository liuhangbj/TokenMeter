// 标准账户卡片：只识别主指标、额度行与余额行，不感知具体供应商。
import { open } from "@tauri-apps/plugin-shell";
import type { AccountCardModel, BalanceCardItem, QuotaCardItem, QuotaUnit } from "./types";
import {
  brandStyleVars, fmtMoney, fmtTokens, levelClass, resetIn, statusDotClass,
  updatedAgo, usageLevel,
} from "./utils";
import { BrandIcon } from "./icons";

function isCurrency(unit: QuotaUnit): unit is { Currency: string } {
  return typeof unit === "object" && "Currency" in unit;
}

function formatAmount(value: number, unit: QuotaUnit): string {
  if (isCurrency(unit)) return fmtMoney(value, unit.Currency);
  if (unit === "Tokens" || unit === "Requests") return fmtTokens(value);
  return `${Math.round(value)}%`;
}

/** 各平台返回精度不一致；额度百分比统一四舍五入为整数展示。 */
function formatPercent(value: number): string {
  return `${Math.round(value)}%`;
}

function quotaUsageText(item: QuotaCardItem): string {
  const pct = item.used_percent;
  if (item.unit === "Percent") return pct === null ? "—" : formatPercent(pct);
  if (item.limit !== null && item.used !== null) {
    const amount = `${formatAmount(item.used, item.unit)} / ${formatAmount(item.limit, item.unit)}`;
    return pct === null ? amount : `${amount} · ${formatPercent(pct)}`;
  }
  if (item.used !== null) return formatAmount(item.used, item.unit);
  if (pct !== null) return formatPercent(pct);
  return "—";
}

function QuotaRow({ item }: { item: QuotaCardItem }) {
  const hasCap = item.limit !== null && item.limit > 0;
  const level = usageLevel(item.used_percent, hasCap);
  const cls = hasCap ? levelClass(level) : "brand";

  return (
    <div className="wrow quota-row">
      <div className="wrow-top">
        <span className="wrow-label">{item.label}</span>
        <span className="wrow-reset">{resetIn(item.reset_at)}</span>
        <span className="wrow-pct tnum">{quotaUsageText(item)}</span>
      </div>
      <div className="bar"><i className={cls} style={{ width: `${item.used_percent ?? 0}%` }} /></div>
    </div>
  );
}

function BalanceRow({ item }: { item: BalanceCardItem }) {
  return (
    <div className="wrow metric-row">
      <div className="wrow-top">
        <span className="wrow-label">{item.label}</span>
        <span className="wrow-reset" />
        <span className="wrow-pct tnum">
          {item.value === null ? "—" : formatAmount(item.value, item.unit)}
        </span>
      </div>
    </div>
  );
}

function planTierClass(tier: number | null): string {
  return tier !== null && tier >= 0 && tier <= 5 ? `tier-${tier}` : "tier-neutral";
}

function primaryValue(card: AccountCardModel): string {
  const { value, unit } = card.primary;
  if (value === null || unit === null) return "—";
  return formatAmount(value, unit);
}

export function ProviderCard({
  card,
  accountLabel,
}: {
  card: AccountCardModel;
  accountLabel?: string;
}) {
  const healthLevel = card.primary.health_used_percent === null
    ? 0
    : usageLevel(card.primary.health_used_percent, true);
  const dotCls = statusDotClass(card.status, healthLevel);
  const quotaItems = card.items.filter((item): item is QuotaCardItem => item.kind === "quota");
  const balanceItems = card.items.filter((item): item is BalanceCardItem => item.kind === "balance");
  const link = card.provider.detail_url;

  return (
    <div
      className="card"
      data-brand={card.provider.brand.key}
      style={brandStyleVars(card.provider.brand)}
    >
      <div className="card-head">
        <span className="brand-icon">
          <BrandIcon brand={card.provider.brand.key} label={card.provider.name} />
        </span>
        <span className="card-identity">
          <span className="card-name">{card.provider.name}</span>
          {accountLabel && <span className="account-label" title={accountLabel}>{accountLabel}</span>}
        </span>
        <span className="spacer" />
        {card.plan && (
          <span className={`plan-badge ${planTierClass(card.plan.tier)}`}>{card.plan.name}</span>
        )}
        <span className={`status-dot ${dotCls}`} title="用量状态" />
      </div>

      <div className="balance-meta">
        <span className="balance-sub">{card.primary.label}</span>
        <span className="updated-at">更新于 {updatedAgo(card.fetched_at)}</span>
      </div>
      <div className="balance-hero tnum">{primaryValue(card)}</div>

      {/* 映射层已决定语义；UI 永远按“全部额度 → 全部余额”排列。 */}
      <div className="card-details">
        {quotaItems.map((item, index) => <QuotaRow key={`quota-${index}-${item.label}`} item={item} />)}
        {balanceItems.map((item, index) => <BalanceRow key={`balance-${index}-${item.label}`} item={item} />)}
      </div>

      <div className="card-foot">
        {card.status === "NetworkError" && card.last_error && (
          <span className="foot-error" title={card.last_error}>刷新失败 · 显示上次数据</span>
        )}
        {card.status === "AuthExpired" && <span className="foot-error">凭证已过期，请重新授权</span>}
        <span className="spacer" />
        {link && (
          <a
            className="detail-link"
            href={link}
            target="_blank"
            rel="noreferrer"
            onClick={(event) => {
              event.preventDefault();
              open(link);
            }}
          >
            查看详情 ↗
          </a>
        )}
      </div>
    </div>
  );
}
