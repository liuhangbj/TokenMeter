import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { AccountCardModel, AppSettings, FloatingOrbSettings, QuotaUnit } from "./types";
import { BrandIcon } from "./icons";
import { fmtTokens, usageLevel } from "./utils";
import { applyTheme } from "./themeRuntime";

function resolveAccountName(card: AccountCardModel, settings: AppSettings | null): string {
  return settings?.account_nicknames?.[card.account_id]?.trim()
    || card.account_label?.trim()
    || card.account_id;
}

function applyOrder(cards: AccountCardModel[], order: string[]): AccountCardModel[] {
  if (order.length === 0) {
    return [...cards].sort((a, b) => {
      const urgency = (card: AccountCardModel) => {
        if (card.status === "Exhausted") return 100;
        if (card.status === "AuthExpired" || card.status === "NetworkError") return 99;
        return card.primary.health_used_percent ?? (card.primary.value !== null ? 1 : 0);
      };
      return urgency(b) - urgency(a);
    });
  }
  const positions = new Map(order.map((id, index) => [id, index]));
  return [...cards].sort((a, b) => {
    const left = positions.get(a.account_id) ?? order.length;
    const right = positions.get(b.account_id) ?? order.length;
    return left - right;
  });
}

function healthLevel(card: AccountCardModel): string {
  if (card.status === "AuthExpired" || card.status === "NetworkError") return "stale";
  if (card.status === "Exhausted") return "lv5";
  if (card.status === "Degraded") return "lv3";
  const percent = card.primary.health_used_percent;
  if (percent === null) return "neutral";
  return `lv${usageLevel(percent, true)}`;
}

function waterLevel(card: AccountCardModel): number | null {
  const percent = card.primary.health_used_percent;
  if (percent === null || !Number.isFinite(percent)) return null;
  return Math.max(3, Math.min(100, percent));
}

function currencySymbol(currency: string): string {
  if (currency === "CNY") return "¥";
  if (currency === "USD") return "$";
  return currency;
}

function compactValue(value: number | null, unit: QuotaUnit | null): string {
  if (value === null || unit === null) return "—";
  if (typeof unit === "object" && "Currency" in unit) {
    const absolute = Math.abs(value);
    const amount = absolute >= 1_000 ? fmtTokens(value) : value.toLocaleString("en-US", {
      maximumFractionDigits: absolute >= 100 ? 0 : absolute >= 10 ? 1 : 2,
    });
    return `${currencySymbol(unit.Currency)}${amount}`;
  }
  if (unit === "Tokens" || unit === "Requests") return fmtTokens(value);
  return `${Math.round(value)}%`;
}

function Orb({
  card,
  large,
  accountName,
  onClick,
  onPointerDown,
  onPointerMove,
  onPointerUp,
  onPointerCancel,
  ariaLabel,
}: {
  card: AccountCardModel;
  large: boolean;
  accountName: string;
  onClick: () => void;
  onPointerDown?: (event: ReactPointerEvent<HTMLButtonElement>) => void;
  onPointerMove?: (event: ReactPointerEvent<HTMLButtonElement>) => void;
  onPointerUp?: (event: ReactPointerEvent<HTMLButtonElement>) => void;
  onPointerCancel?: (event: ReactPointerEvent<HTMLButtonElement>) => void;
  ariaLabel?: string;
}) {
  const water = waterLevel(card);
  const value = compactValue(card.primary.value, card.primary.unit);
  return (
    <button
      className={`floating-orb ${large ? "large" : "small"} ${healthLevel(card)}`}
      type="button"
      onClick={onClick}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerCancel}
      aria-label={ariaLabel ?? (large
        ? `${card.provider.name} ${accountName} ${value}，打开主面板`
        : `切换到 ${card.provider.name} ${accountName} ${value}`)}
      title={`${card.provider.name} · ${accountName} · ${card.primary.label} ${value}`}
    >
      <span className="floating-orb-planet">
        {water !== null && (
          <span
            className="floating-orb-water"
            style={{ "--orb-water": `${water}%` } as CSSProperties}
          />
        )}
        {water === null && <span className="floating-orb-balance-ring" />}
        <span className="floating-orb-brand">
          <BrandIcon
            brand={card.provider.brand.key}
            label={card.provider.name}
            size={large ? 76 : 31}
          />
        </span>
        <span className="floating-orb-value tnum">{value}</span>
      </span>
    </button>
  );
}

export default function FloatingOrb() {
  const [cards, setCards] = useState<AccountCardModel[]>([]);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [edge, setEdge] = useState<"left" | "right">("right");
  const [focused, setFocused] = useState(false);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null);
  const dragSettleTimer = useRef<number | null>(null);
  const dragIsUserInitiated = useRef(false);
  const dragStartedCollapsed = useRef(false);
  const collapsedLastMovedAt = useRef(0);
  const collapsedPointer = useRef<{ pointerId: number; x: number; y: number } | null>(null);
  const layoutAccountCount = useRef<number | null>(null);

  const loadCards = useCallback(async () => {
    const next = await invoke<AccountCardModel[]>("get_account_cards");
    setCards(next);
  }, []);

  useEffect(() => {
    Promise.all([
      invoke<AppSettings>("get_settings"),
      invoke<AccountCardModel[]>("get_account_cards"),
    ]).then(([loadedSettings, loadedCards]) => {
      applyTheme(loadedSettings.theme, loadedSettings.appearance);
      setSettings(loadedSettings);
      setEdge(loadedSettings.floating_orb.edge);
      setCards(loadedCards);
    }).catch(console.error);

    const snapshots = listen("snapshots-updated", () => loadCards().catch(console.error));
    const settingsChanged = listen<AppSettings>("settings-updated", ({ payload }) => {
      applyTheme(payload.theme, payload.appearance);
      setSettings(payload);
      setEdge(payload.floating_orb.edge);
    });
    return () => {
      snapshots.then((dispose) => dispose());
      settingsChanged.then((dispose) => dispose());
    };
  }, [loadCards]);

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    currentWindow.isFocused().then(setFocused).catch(console.error);
    const unlisten = currentWindow.onFocusChanged(({ payload }) => {
      setFocused(payload);
      if (!payload) setContextMenu(null);
    });
    return () => { unlisten.then((dispose) => dispose()); };
  }, []);

  useEffect(() => {
    const unlisten = getCurrentWindow().onMoved(() => {
      // set_size/set_position 也会产生 Moved。只为用户从拖动手势发起的移动
      // 执行边缘吸附，避免 Windows 启动或 DPI 调整时重复改写显示器位置。
      if (!dragIsUserInitiated.current) return;
      if (dragStartedCollapsed.current) {
        collapsedLastMovedAt.current = Date.now();
      }
      if (dragSettleTimer.current !== null) window.clearTimeout(dragSettleTimer.current);
      dragSettleTimer.current = window.setTimeout(async () => {
        try {
          const snapped = await invoke<FloatingOrbSettings>("snap_floating_orb");
          setEdge(snapped.edge);
        } catch (error) {
          console.error("吸附悬浮球失败", error);
        } finally {
          dragSettleTimer.current = null;
          dragIsUserInitiated.current = false;
          dragStartedCollapsed.current = false;
        }
      }, 220);
    });
    return () => {
      if (dragSettleTimer.current !== null) window.clearTimeout(dragSettleTimer.current);
      unlisten.then((dispose) => dispose());
    };
  }, []);

  const ordered = useMemo(
    () => applyOrder(cards, settings?.card_order ?? []),
    [cards, settings?.card_order],
  );
  const configuredActiveId = settings?.floating_orb.active_account_id;
  const active = ordered.find((card) => card.account_id === configuredActiveId) ?? ordered[0];
  const inactive = active ? ordered.filter((card) => card.account_id !== active.account_id) : [];
  const collapsed = settings?.floating_orb.collapsed ?? false;

  useEffect(() => {
    if (!settings || layoutAccountCount.current === ordered.length) return;
    layoutAccountCount.current = ordered.length;
    invoke("sync_floating_orb_layout", { accountCount: ordered.length }).catch((error) => {
      layoutAccountCount.current = null;
      console.error("同步悬浮球尺寸失败", error);
    });
  }, [ordered.length, settings]);

  const selectAccount = async (accountId: string) => {
    setSettings((current) => current ? {
      ...current,
      floating_orb: { ...current.floating_orb, active_account_id: accountId },
    } : current);
    try {
      const saved = await invoke<AppSettings>("set_floating_orb_active_account", { accountId });
      setSettings(saved);
    } catch (error) {
      console.error("保存悬浮球激活账号失败", error);
    }
  };

  const startDrag = async (startedCollapsed = false) => {
    dragIsUserInitiated.current = true;
    dragStartedCollapsed.current = startedCollapsed;
    try {
      await getCurrentWindow().startDragging();
    } catch (error) {
      dragIsUserInitiated.current = false;
      dragStartedCollapsed.current = false;
      console.error("拖动悬浮球失败", error);
    }
  };

  const beginCollapsedGesture = (event: ReactPointerEvent<HTMLButtonElement>) => {
    if (event.button !== 0) return;
    // 鼠标快速移出小球时仍需收到首个 pointermove；Windows 自动化和高 DPI
    // 鼠标都可能一次跨过数十像素，不能依赖指针始终停留在按钮命中区内。
    event.currentTarget.setPointerCapture(event.pointerId);
    collapsedPointer.current = {
      pointerId: event.pointerId,
      x: event.screenX,
      y: event.screenY,
    };
  };

  const continueCollapsedGesture = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const origin = collapsedPointer.current;
    if (!origin || origin.pointerId !== event.pointerId || dragIsUserInitiated.current) return;
    if ((event.buttons & 1) === 0) return;
    const distance = Math.hypot(event.screenX - origin.x, event.screenY - origin.y);
    if (distance < 5) return;
    // 普通点击不调用系统拖动；只有越过阈值才交给原生窗口移动。
    // 这样 Windows 不会因为 WM_NCLBUTTONDOWN 吞掉 click 而无法展开小球。
    collapsedLastMovedAt.current = Date.now();
    void startDrag(true);
  };

  const endCollapsedGesture = (event: ReactPointerEvent<HTMLButtonElement>) => {
    if (collapsedPointer.current?.pointerId === event.pointerId) {
      collapsedPointer.current = null;
    }
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const expandCollapsed = () => {
    if (Date.now() - collapsedLastMovedAt.current < 400) {
      return;
    }
    void setCollapsed(false);
  };

  const openActive = () => {
    if (!active) return;
    invoke("open_account_from_floating_orb", { accountId: active.account_id }).catch(console.error);
  };

  const setCollapsed = async (nextCollapsed: boolean) => {
    const previous = settings;
    if (previous) setSettings({
      ...previous,
      floating_orb: { ...previous.floating_orb, collapsed: nextCollapsed },
    });
    try {
      const saved = await invoke<AppSettings>("set_floating_orb_collapsed", {
        collapsed: nextCollapsed,
        accountCount: ordered.length,
      });
      setSettings(saved);
    } catch (error) {
      setSettings(previous);
      console.error("切换悬浮球收起状态失败", error);
    }
  };

  const hideFloatingOrb = () => {
    setContextMenu(null);
    invoke("hide_floating_orb").catch((error) => {
      console.error("隐藏悬浮组件失败", error);
    });
  };

  const openContextMenu = (event: ReactMouseEvent<HTMLElement>) => {
    event.preventDefault();
    const width = 72;
    const height = 38;
    setContextMenu({
      x: Math.max(4, Math.min(event.clientX, window.innerWidth - width - 4)),
      y: Math.max(4, Math.min(event.clientY, window.innerHeight - height - 4)),
    });
  };

  const contextMenuElement = contextMenu && (
    <div
      className="floating-orb-context"
      role="menu"
      style={{ left: contextMenu.x, top: contextMenu.y }}
      onPointerDown={(event) => event.stopPropagation()}
    >
      <button type="button" role="menuitem" onClick={hideFloatingOrb}>隐藏</button>
    </div>
  );

  if (collapsed) {
    return (
      <main
        className="floating-orb-stage collapsed"
        data-edge={edge}
        data-focused={focused}
        onContextMenu={openContextMenu}
        onPointerDown={() => setContextMenu(null)}
      >
        {active ? (
          <Orb
            card={active}
            large={false}
            accountName={resolveAccountName(active, settings)}
            onClick={expandCollapsed}
            onPointerDown={beginCollapsedGesture}
            onPointerMove={continueCollapsedGesture}
            onPointerUp={endCollapsedGesture}
            onPointerCancel={endCollapsedGesture}
            ariaLabel={`展开 ${active.provider.name} ${resolveAccountName(active, settings)} 悬浮球`}
          />
        ) : (
          <button
            type="button"
            className="floating-orb-collapsed-empty"
            onClick={expandCollapsed}
            onPointerDown={beginCollapsedGesture}
            onPointerMove={continueCollapsedGesture}
            onPointerUp={endCollapsedGesture}
            onPointerCancel={endCollapsedGesture}
            aria-label="展开 TokenMeter 悬浮球"
          >···</button>
        )}
        {contextMenuElement}
      </main>
    );
  }

  return (
    <main
      className="floating-orb-stage"
      data-edge={edge}
      data-focused={focused}
      onContextMenu={openContextMenu}
      onPointerDown={() => setContextMenu(null)}
    >
      <section className="floating-orb-rail" aria-label="TokenMeter 账户悬浮球">
        <button
          type="button"
          className="floating-orb-grip"
          onPointerDown={(event) => {
            event.preventDefault();
            void startDrag(false);
          }}
          aria-label="拖动悬浮球"
          title="拖动并吸附到屏幕边缘"
        ><span /></button>
        <button
          type="button"
          className="floating-orb-collapse"
          onClick={() => void setCollapsed(true)}
          aria-label="收起悬浮球，只显示当前主账号"
          title="收起为主账号小球"
        ><span /></button>

        {active ? (
          <>
            <div className="floating-orb-main">
              <Orb
                card={active}
                large
                accountName={resolveAccountName(active, settings)}
                onClick={openActive}
              />
              <span className="floating-orb-account" title={resolveAccountName(active, settings)}>
                {active.provider.name} · {resolveAccountName(active, settings)}
              </span>
            </div>
            <div className="floating-orb-list">
              {inactive.map((card) => (
                <Orb
                  key={card.account_id}
                  card={card}
                  large={false}
                  accountName={resolveAccountName(card, settings)}
                  onClick={() => void selectAccount(card.account_id)}
                />
              ))}
            </div>
            <span className="floating-orb-sync">LIVE</span>
          </>
        ) : (
          <div className="floating-orb-empty" title="等待账户数据">···</div>
        )}
      </section>
      {contextMenuElement}
    </main>
  );
}
