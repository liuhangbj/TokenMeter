// 托盘下拉面板主组件
import { useEffect, useState, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ProviderSnapshot } from "./types";
import { ProviderCard } from "./ProviderCard";
import { SettingsPanel } from "./SettingsPanel";
import { AddProvider } from "./AddProvider";
import { autoCheckOnLaunch } from "./updater";
import { IconSettings, IconSort, IconCheck, IconRefresh } from "./icons";

interface Settings {
  launch_at_login: boolean;
  refresh_interval_secs: number;
  card_order: string[];
}

/** 按"紧张度"降序排序（默认）。无窗口的按余额可用性排后。 */
function sortByUrgency(snaps: ProviderSnapshot[]): ProviderSnapshot[] {
  return [...snaps].sort((a, b) => urgency(b) - urgency(a));
}

function urgency(s: ProviderSnapshot): number {
  const maxPct = Math.max(0, ...s.windows.map((w) => w.used ?? 0));
  if (maxPct > 0) return maxPct;
  if (s.balance) return s.balance.available ? 1 : 50;
  return 0;
}

/** 按用户自定义顺序排序；未在顺序表里的追加到末尾（保持原相对序）。 */
function applyCustomOrder(snaps: ProviderSnapshot[], order: string[]): ProviderSnapshot[] {
  if (order.length === 0) return snaps;
  const idx = new Map(order.map((id, i) => [id, i]));
  return [...snaps].sort((a, b) => {
    const ia = idx.has(a.provider_id) ? idx.get(a.provider_id)! : order.length;
    const ib = idx.has(b.provider_id) ? idx.get(b.provider_id)! : order.length;
    return ia - ib;
  });
}

export default function App() {
  const [snaps, setSnaps] = useState<ProviderSnapshot[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [customOrder, setCustomOrder] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [spinning, setSpinning] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [editMode, setEditMode] = useState(false);
  const [configured, setConfigured] = useState(false); // 是否已配置过 provider
  const [gotUpdate, setGotUpdate] = useState(false);    // 是否收到过一次刷新完成事件
  const [view, setView] = useState<"home" | "add">("home"); // 内嵌视图：主面板 / 添加供应商

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
      const data = await invoke<ProviderSnapshot[]>("get_snapshots");
      setSnaps(data);
      if (data.length === 0) {
        // 首屏防闪烁：启动时快照还没抓完，先确认是否已有配置，
        // 避免把"正在获取"误显示成"还没有添加供应商"。
        const has = await invoke<boolean>("has_configured_providers").catch(() => false);
        setConfigured(has);
      }
    } catch (e) {
      console.error("get_snapshots 失败", e);
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
      localStorage.setItem(KEY, String(now));
      autoCheckOnLaunch();
    }
  }, []);

  // 自适应窗口尺寸：监听内容节点（.popover-body / .wizard）而非滚动容器，
  // 否则内容在 max-height 内增长时 ResizeObserver 不会触发（窗口高度卡在旧值）。
  // 宽度随视图变化（主面板 380 / 添加供应商向导约 506），高度随内容走（封顶 800）。
  const lastSize = useRef({ w: 0, h: 0 });
  useEffect(() => {
    const el = document.querySelector(".popover");
    if (!el) return;
    const apply = () => {
      let w = 380;
      if (view === "add") {
        const wizard = document.querySelector(".wizard");
        const wizardW = wizard ? Math.ceil(wizard.getBoundingClientRect().width) : 480;
        // popover 左右 padding 12*2 + border 1*2
        w = wizardW + 26;
      }
      const h = Math.ceil(el.scrollHeight);
      if (w === lastSize.current.w && h === lastSize.current.h) return;
      lastSize.current = { w, h };
      invoke("resize_popover", { width: w, height: h }).catch(() => {});
    };
    const raf = requestAnimationFrame(apply);
    const ro = new ResizeObserver(() => requestAnimationFrame(apply));
    const targets: Element[] = [];
    const body = document.querySelector(".popover-body");
    const wizard = document.querySelector(".wizard");
    if (body) targets.push(body);
    if (wizard) targets.push(wizard);
    targets.forEach((t) => ro.observe(t));
    // 字体/异步渲染兜底：晚到的布局变化也能补一次测量
    const t1 = window.setTimeout(apply, 500);
    if (document.fonts?.ready) {
      document.fonts.ready.then(() => requestAnimationFrame(apply)).catch(() => {});
    }
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      clearTimeout(t1);
    };
  }, [view]);
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

  const onRemove = async (id: string) => {
    const name = snaps.find((s) => s.provider_id === id)?.display_name ?? id;
    if (!window.confirm(`确定移除「${name}」吗？将删除已保存的凭证。`)) return;
    try {
      await invoke("remove_provider", { providerId: id });
      load();
    } catch (e) {
      console.error("移除供应商失败", e);
    }
  };

  // ---- 上下箭头排序（比拖拽更可靠，菜单栏小面板拖拽易被拦截）----
  const persistOrder = async (order: string[]) => {
    if (!settings) return;
    const next = { ...settings, card_order: order };
    setSettings(next);
    try {
      await invoke("set_settings", { settings: next });
    } catch (e) {
      console.error("保存排序失败", e);
    }
  };

  /** 把 provider_id 在 displayed 里上移/下移一位。 */
  const move = (id: string, dir: -1 | 1) => {
    const current = displayed.map((s) => s.provider_id);
    const idx = current.indexOf(id);
    const target = idx + dir;
    if (idx < 0 || target < 0 || target >= current.length) return;
    const next = [...current];
    [next[idx], next[target]] = [next[target], next[idx]];
    setCustomOrder(next);
    persistOrder(next);
  };

  // 显示顺序：有自定义顺序用自定义，否则按紧张度
  const displayed = customOrder.length > 0 ? applyCustomOrder(snaps, customOrder) : sortByUrgency(snaps);

  return (
    <div className="popover">
      {view === "add" ? (
        <AddProvider onDone={() => setView("home")} />
      ) : (
        <div className="popover-body">
          <div className="popover-head">
            <span className="popover-title">TokenMeter</span>
            <span className="spacer" />
            {snaps.length > 0 && (
              <button
                className={`icon-btn ${editMode ? "active" : ""}`}
                onClick={() => setEditMode((v) => !v)}
                title={editMode ? "完成排序" : "自定义排序"}
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

          {loading ? (
            <div className="empty">加载中…</div>
          ) : snaps.length === 0 ? (
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
              <div key={s.provider_id} className={`card-drag-wrap ${editMode ? "editable" : ""}`}>
                {editMode && (
                  <div className="sort-arrows">
                    <button
                      className="sort-btn"
                      disabled={i === 0}
                      onClick={() => move(s.provider_id, -1)}
                      title="上移"
                    >
                      ↑
                    </button>
                    <button
                      className="sort-btn"
                      disabled={i === displayed.length - 1}
                      onClick={() => move(s.provider_id, 1)}
                      title="下移"
                    >
                      ↓
                    </button>
                    <button
                      className="sort-btn danger"
                      onClick={() => onRemove(s.provider_id)}
                      title="移除供应商"
                    >
                      ✕
                    </button>
                  </div>
                )}
                <ProviderCard snap={s} />
              </div>
            ))
          )}

          <div className="popover-foot">
            <button className="btn primary" onClick={onAdd}>
              + 添加供应商
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
