// 供应商卡片 —— 按数据形态自适应渲染（额度型 / 余额型 / 套餐包）
import type { ProviderSnapshot, QuotaWindow } from "./types";
import {
  usageLevel, levelClass, statusDotClass, fidMeta,
  resetIn, updatedAgo, fmtMoney, brandOf, consoleUrl,
} from "./utils";

function WindowRow({ w }: { w: QuotaWindow }) {
  const hasCap = w.limit !== null && w.limit > 0;
  const lv = usageLevel(w.used, hasCap);
  const cls = hasCap ? levelClass(lv) : "brand";
  const pct = w.used ?? 0;

  // 货币单位（如 Extra Usage 钱包）：显示"已用/限额 ¥X/¥Y"；百分比单位显示百分比
  const isCurrency = typeof w.unit === "object" && "Currency" in w.unit;
  const currency = isCurrency ? (w.unit as { Currency: string }).Currency : null;
  const rightText = isCurrency && currency && w.limit !== null
    ? `${fmtMoney(w.limit - (w.remaining ?? w.limit), currency)} / ${fmtMoney(w.limit, currency)}`
    : w.used !== null
      ? `${Math.round(w.used)}%`
      : "—";

  return (
    <div className="wrow">
      <div className="wrow-top">
        <span className="wrow-label">{w.label}</span>
        <span className="wrow-reset">{resetIn(w.reset_at)}</span>
        <span className="wrow-pct tnum">{rightText}</span>
      </div>
      <div className="bar"><i className={cls} style={{ width: `${pct}%` }} /></div>
    </div>
  );
}

export function ProviderCard({ snap }: { snap: ProviderSnapshot }) {
  const brand = brandOf(snap.provider_id);
  const maxLv = Math.max(0, ...snap.windows.map((w) => usageLevel(w.used, w.limit !== null && w.limit > 0)));
  const dotCls = statusDotClass(snap.status, maxLv);
  const [fidCls, fidLabel] = fidMeta(snap.fidelity);
  const link = consoleUrl(snap.provider_id);

  return (
    <div className="card" data-brand={brand}>
      <div className="card-head">
        <span className="brand-dot" />
        <span className="card-name">{snap.display_name}</span>
        {snap.plan_name && <span className="plan-badge">{snap.plan_name}</span>}
        <span className="spacer" />
        <span className={`status-dot ${dotCls}`} title="用量状态" />
      </div>

      {/* 余额 hero（按量制） */}
      {snap.balance && (
        <>
          <div className="balance-hero tnum">{fmtMoney(snap.balance.total, snap.balance.currency)}</div>
          <div className="balance-sub">
            {snap.balance.topped_up !== null && `充值 ${fmtMoney(snap.balance.topped_up, snap.balance.currency)}`}
            {snap.balance.granted !== null && snap.balance.granted > 0 && ` · 赠送 ${fmtMoney(snap.balance.granted, snap.balance.currency)}`}
            {!snap.balance.available && " · 余额不足"}
          </div>
        </>
      )}

      {/* 额度窗口（订阅制） */}
      {snap.windows.map((w, i) => (
        <WindowRow key={i} w={w} />
      ))}

      {/* 卡脚 */}
      <div className="card-foot">
        <span className={`fid-dot ${fidCls}`} title={fidLabel} />
        <span>{updatedAgo(snap.fetched_at)}</span>
        <span className="spacer" />
        {link && (
          <a className="detail-link" href={link} target="_blank" rel="noreferrer">
            查看详情 ↗
          </a>
        )}
      </div>
    </div>
  );
}
