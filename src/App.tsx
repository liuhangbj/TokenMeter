// 托盘下拉面板主组件
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { AccountCardModel } from "./types";
import { ProviderCard } from "./ProviderCard";
import { SettingsPanel } from "./SettingsPanel";
import { AddProvider } from "./AddProvider";
import { autoCheckOnLaunch } from "./updater";
import { IconSettings, IconSort, IconCheck, IconRefresh, IconGauge } from "./icons";

/** 与后端 PANEL_W / PANEL_H 保持一致。 */
const PANEL_WIDTH = 380;
const PANEL_MAX_HEIGHT = 800;
const IS_WINDOWS = navigator.userAgent.includes("Windows");

// WebView2 使用不透明宿主窗口；CSS 需要知道平台以避免透明圆角等 macOS 专属效果。
document.documentElement.dataset.platform = IS_WINDOWS ? "windows" : "macos";

interface Settings {
  launch_at_login: boolean;
  refresh_interval_secs: number;
  card_order: string[];
  account_nicknames: Record<string, string>;
}

export function resolveAccountName(
  accountId: string,
  returnedLabel: string | null | undefined,
  nicknames: Record<string, string> | null | undefined,
): string {
  return nicknames?.[accountId]?.trim() || returnedLabel?.trim() || accountId;
}

export function withAccountNickname(settings: Settings, accountId: string, draft: string): Settings {
  const account_nicknames = { ...(settings.account_nicknames ?? {}) };
  const nickname = draft.trim();
  if (nickname) account_nicknames[accountId] = nickname;
  else delete account_nicknames[accountId];
  return { ...settings, account_nicknames };
}

function withoutAccountReferences(settings: Settings, accountId: string): Settings {
  const account_nicknames = { ...(settings.account_nicknames ?? {}) };
  delete account_nicknames[accountId];
  return {
    ...settings,
    card_order: (settings.card_order ?? []).filter((id) => id !== accountId),
    account_nicknames,
  };
}

/** 按"紧张度"降序排序（默认）。无窗口的按余额可用性排后。 */
function sortByUrgency(cards: AccountCardModel[]): AccountCardModel[] {
  return [...cards].sort((a, b) => urgency(b) - urgency(a));
}

function urgency(card: AccountCardModel): number {
  const used = card.primary.health_used_percent ?? 0;
  if (used > 0) return used;
  if (card.status === "Exhausted") return 100;
  if (card.primary.value !== null) return 1;
  return 0;
}

/** 按用户自定义顺序排序；未在顺序表里的追加到末尾（保持原相对序）。 */
function applyCustomOrder(cards: AccountCardModel[], order: string[]): AccountCardModel[] {
  if (order.length === 0) return cards;
  const idx = new Map(order.map((id, i) => [id, i]));
  return [...cards].sort((a, b) => {
    const ia = idx.has(a.account_id) ? idx.get(a.account_id)! : order.length;
    const ib = idx.has(b.account_id) ? idx.get(b.account_id)! : order.length;
    return ia - ib;
  });
}

export default function App() {
  const panelRef = useRef<HTMLDivElement>(null);
  const fixedRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const lastPanelHeight = useRef<number | null>(null);
  const [cards, setCards] = useState<AccountCardModel[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [customOrder, setCustomOrder] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [spinning, setSpinning] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [editMode, setEditMode] = useState(false);
  const [renamingAccountId, setRenamingAccountId] = useState<string | null>(null);
  const [removingAccountId, setRemovingAccountId] = useState<string | null>(null);
  const [removeBusyAccountId, setRemoveBusyAccountId] = useState<string | null>(null);
  const [nicknameDraft, setNicknameDraft] = useState("");
  const nicknameInputRef = useRef<HTMLInputElement>(null);
  const [configured, setConfigured] = useState(false); // 是否已配置过 provider
  const [gotUpdate, setGotUpdate] = useState(false);    // 是否收到过一次刷新完成事件
  const [view, setView] = useState<"home" | "add">("home"); // 内嵌视图：主面板 / 添加供应商

  // 固定区（标题 + 设置）不参与滚动；内容区独立滚动。窗口仍按两区自然高度
  // 自适应收缩，超过 800px 时只压缩内容区。Windows 保持固定外层高度，
  // 避免 WebView2 在可见状态下 resize 导致定位漂移与闪切。
  useLayoutEffect(() => {
    if (IS_WINDOWS) return;
    const panel = panelRef.current;
    const content = contentRef.current;
    if (!panel || !content) return;

    let frame: number | null = null;
    const resize = () => {
      frame = null;
      const style = window.getComputedStyle(panel);
      const borders =
        Number.parseFloat(style.borderTopWidth) + Number.parseFloat(style.borderBottomWidth);
      const paddings =
        Number.parseFloat(style.paddingTop) + Number.parseFloat(style.paddingBottom);
      const fixedHeight = fixedRef.current?.getBoundingClientRect().height ?? 0;
      const contentHeight = content.scrollHeight;
      const height = Math.min(
        PANEL_MAX_HEIGHT,
        Math.ceil(fixedHeight + contentHeight + paddings + borders),
      );

      if (lastPanelHeight.current === height) return;
      lastPanelHeight.current = height;
      getCurrentWindow()
        .setSize(new LogicalSize(PANEL_WIDTH, height))
        .catch((error) => console.error("调整面板尺寸失败", error));
    };
    const scheduleResize = () => {
      if (frame !== null) window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(resize);
    };

    const observer = new ResizeObserver(scheduleResize);
    observer.observe(panel);
    observer.observe(content);
    if (fixedRef.current) observer.observe(fixedRef.current);
    scheduleResize();

    return () => {
      observer.disconnect();
      if (frame !== null) window.cancelAnimationFrame(frame);
    };
  }, [view, showSettings]);

  // 加载设置（拿 card_order）
  useEffect(() => {
    invoke<Settings>("get_settings")
      .then((s) => {
        setSettings(s);
        setCustomOrder(s.card_order ?? []);
      })
      .catch(console.error);
  }, []);

  const load = useCallback(async () => {
    try {
      const data = await invoke<AccountCardModel[]>("get_account_cards");
      setCards(data);
      if (data.length === 0) {
        // 首屏防闪烁：启动时快照还没抓完，先确认是否已有配置，
        // 避免把"正在获取"误显示成"还没有添加供应商"。
        const has = await invoke<boolean>("has_configured_providers").catch(() => false);
        setConfigured(has);
      }
    } catch (e) {
      console.error("get_account_cards 失败", e);
    } finally {
      setLoading(false);
      setSpinning(false);
    }
  }, []);

  useEffect(() => {
    invoke("on_panel_open").catch(console.error);
    load();
  }, [load]);

  // 静默检查更新（有新版则后台自动下载，设置区可见"重启完成更新"）。
  // 面板窗口按需重建（纯菜单栏架构），每次打开都会重新加载前端，
  // 用 localStorage 防抖：24 小时内只检查一次，避免频繁请求 GitHub。
  useEffect(() => {
    const KEY = "tm_last_update_check";
    const now = Date.now();
    const last = Number(localStorage.getItem(KEY) ?? 0);
    if (now - last > 24 * 3600 * 1000) {
      autoCheckOnLaunch().then((success) => {
        if (success) localStorage.setItem(KEY, String(Date.now()));
      });
    }
  }, []);

  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === "visible") {
        invoke("on_panel_open").catch(console.error);
        setTimeout(load, 800);
      }
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onVisible);
    const unlisten = listen("snapshots-updated", () => {
      setGotUpdate(true);
      load();
    });
    return () => {
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", onVisible);
      unlisten.then((f) => f());
    };
  }, [load]);

  // 调试钩子：后端 TOKENMETER_AUTO_PANEL=1 启动时自动进入"添加供应商"视图
  useEffect(() => {
    const un = listen("debug-auto-panel", () => setView("add"));
    return () => {
      un.then((f) => f());
    };
  }, []);

  const onRefresh = () => {
    setSpinning(true);
    invoke("on_panel_open").catch(console.error);
    setTimeout(load, 800);
  };

  const onAdd = () => setView("add");

  const defaultAccountName = (accountId: string) => {
    const returned = cards.find((card) => card.account_id === accountId)?.account_label;
    return resolveAccountName(accountId, returned, null);
  };

  /** 显示优先级：自定义别名 → 平台选出的用户名/邮箱/ID → 本地账号主键。 */
  const accountName = (accountId: string) => {
    const returned = cards.find((card) => card.account_id === accountId)?.account_label;
    return resolveAccountName(accountId, returned, settings?.account_nicknames);
  };

  const onRemove = async (accountId: string) => {
    setRemoveBusyAccountId(accountId);
    try {
      await invoke("remove_provider", { accountId });
      setCards((current) => current.filter((card) => card.account_id !== accountId));
      setCustomOrder((current) => current.filter((id) => id !== accountId));
      setSettings((current) => current ? withoutAccountReferences(current, accountId) : current);
      setRemovingAccountId(null);
      await load();
    } catch (e) {
      console.error("移除供应商失败", e);
    } finally {
      setRemoveBusyAccountId(null);
    }
  };

  const beginRename = (accountId: string) => {
    setRemovingAccountId(null);
    setNicknameDraft(settings?.account_nicknames?.[accountId] ?? "");
    setRenamingAccountId(accountId);
  };

  const beginRemove = (accountId: string) => {
    setRenamingAccountId(null);
    setRemovingAccountId(accountId);
  };

  useEffect(() => {
    if (!renamingAccountId) return;
    const frame = window.requestAnimationFrame(() => {
      nicknameInputRef.current?.focus();
      nicknameInputRef.current?.select();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [renamingAccountId]);

  const saveNickname = async (accountId: string) => {
    const previous = settings;
    if (previous) setSettings(withAccountNickname(previous, accountId, nicknameDraft));
    setRenamingAccountId(null);
    try {
      const saved = await invoke<Settings>("set_account_nickname", {
        accountId,
        nickname: nicknameDraft,
      });
      setSettings(saved);
    } catch (e) {
      console.error("保存账号昵称失败", e);
      setSettings(previous);
      setRenamingAccountId(accountId);
    }
  };

  // ---- 上下箭头排序（比拖拽更可靠，菜单栏小面板拖拽易被拦截）----
  const persistOrder = async (order: string[]) => {
    const previous = settings;
    if (previous) setSettings({ ...previous, card_order: order });
    try {
      const saved = await invoke<Settings>("set_card_order", { cardOrder: order });
      setSettings(saved);
    } catch (e) {
      console.error("保存排序失败", e);
      setSettings(previous);
      setCustomOrder(previous?.card_order ?? []);
    }
  };

  /** 把账号实例在 displayed 里上移/下移一位。 */
  const move = (id: string, dir: -1 | 1) => {
    const current = displayed.map((s) => s.account_id);
    const idx = current.indexOf(id);
    const target = idx + dir;
    if (idx < 0 || target < 0 || target >= current.length) return;
    const next = [...current];
    [next[idx], next[target]] = [next[target], next[idx]];
    setCustomOrder(next);
    persistOrder(next);
  };

  // 显示顺序：有自定义顺序用自定义，否则按紧张度
  const displayed = customOrder.length > 0 ? applyCustomOrder(cards, customOrder) : sortByUrgency(cards);

  return (
    <div ref={panelRef} className="popover">
      {view === "add" ? (
        <div className="popover-scroll popover-scroll-add">
          <div ref={contentRef} className="popover-content">
            <AddProvider onDone={() => setView("home")} />
          </div>
        </div>
      ) : (
        <div className="popover-body">
          <div ref={fixedRef} className="popover-fixed">
            <div className="popover-head">
              <span className="popover-mark"><IconGauge /></span>
              <span className="popover-title">TokenMeter</span>
              <span className="spacer" />
              {cards.length > 0 && (
                <button
                  className={`icon-btn ${editMode ? "active" : ""}`}
                  onClick={() => {
                    setEditMode((v) => !v);
                    setRenamingAccountId(null);
                    setRemovingAccountId(null);
                  }}
                  title={editMode ? "完成编辑" : "编辑账号"}
                >
                  {editMode ? <IconCheck /> : <IconSort />}
                </button>
              )}
              <button className="icon-btn" onClick={() => setShowSettings((v) => !v)} title="设置">
                <IconSettings />
              </button>
              <button className="icon-btn" onClick={onRefresh} title="刷新">
                <span className={spinning ? "spin" : ""} style={{ display: "inline-flex" }}>
                  <IconRefresh />
                </span>
              </button>
            </div>

            {showSettings && <SettingsPanel />}
          </div>

          <div className="popover-scroll">
            <div ref={contentRef} className="popover-content">

            {loading ? (
            <div className="empty">加载中…</div>
          ) : cards.length === 0 ? (
            <div className="empty">
              {configured && !gotUpdate ? (
                "正在获取额度数据…"
              ) : (
                <>
                  还没有添加供应商
                  <br />
                  点击下方按钮开始
                </>
              )}
            </div>
          ) : (
            displayed.map((s, i) => (
              <div
                key={s.account_id}
                className={`card-drag-wrap ${editMode ? "editable" : ""} ${renamingAccountId === s.account_id ? "renaming" : ""} ${removingAccountId === s.account_id ? "removing" : ""}`}
              >
                {editMode && renamingAccountId === s.account_id && (
                  <form
                    className="nickname-editor"
                    onSubmit={(event) => {
                      event.preventDefault();
                      saveNickname(s.account_id);
                    }}
                  >
                    <input
                      ref={nicknameInputRef}
                      value={nicknameDraft}
                      onChange={(event) => setNicknameDraft(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Escape") setRenamingAccountId(null);
                      }}
                      placeholder={`默认：${defaultAccountName(s.account_id)}`}
                      aria-label="账号别名"
                    />
                    <button className="sort-btn save" type="submit" title="保存别名">✓</button>
                    <button
                      className="sort-btn"
                      type="button"
                      onClick={() => setRenamingAccountId(null)}
                      title="取消"
                    >
                      ×
                    </button>
                  </form>
                )}
                {editMode && removingAccountId === s.account_id && (
                  <div className="remove-confirm" role="alert">
                    <span className="remove-confirm-label" title={`${s.provider.name} · ${accountName(s.account_id)}`}>
                      删除 {s.provider.name}？
                    </span>
                    <button
                      className="remove-confirm-btn danger"
                      type="button"
                      disabled={removeBusyAccountId === s.account_id}
                      onClick={() => onRemove(s.account_id)}
                    >
                      {removeBusyAccountId === s.account_id ? "删除中…" : "确认删除"}
                    </button>
                    <button
                      className="remove-confirm-btn"
                      type="button"
                      disabled={removeBusyAccountId === s.account_id}
                      onClick={() => setRemovingAccountId(null)}
                    >
                      取消
                    </button>
                  </div>
                )}
                {editMode
                  && renamingAccountId !== s.account_id
                  && removingAccountId !== s.account_id && (
                  <div className="sort-arrows">
                    <button
                      className="sort-btn"
                      type="button"
                      onClick={() => beginRename(s.account_id)}
                      title="设置账号昵称"
                    >
                      ✎
                    </button>
                    <button
                      className="sort-btn"
                      type="button"
                      disabled={i === 0}
                      onClick={() => move(s.account_id, -1)}
                      title="上移"
                    >
                      ↑
                    </button>
                    <button
                      className="sort-btn"
                      type="button"
                      disabled={i === displayed.length - 1}
                      onClick={() => move(s.account_id, 1)}
                      title="下移"
                    >
                      ↓
                    </button>
                    <button
                      className="sort-btn danger"
                      type="button"
                      onClick={() => beginRemove(s.account_id)}
                      title="移除供应商"
                    >
                      ✕
                    </button>
                  </div>
                )}
                <ProviderCard card={s} accountLabel={accountName(s.account_id)} />
              </div>
            ))
          )}

              <div className="popover-foot">
                <button className="btn primary" onClick={onAdd}>
                  + 添加供应商
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
