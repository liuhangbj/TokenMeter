// 设置区：开机启动勾选 + 后台刷新间隔下拉 + 检查更新 + 退出应用
import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import {
  subscribeUpdate,
  checkForUpdate,
  download,
  installAndRelaunch,
  type UpdateState,
} from "./updater";
import type { AppSettings as Settings } from "./types";
import { applyTheme } from "./themeRuntime";

const INTERVAL_LABELS: Record<number, string> = {
  60: "1 分钟",
  180: "3 分钟",
  300: "5 分钟",
  600: "10 分钟",
  900: "15 分钟",
  1800: "30 分钟",
};

export function SettingsPanel() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [options, setOptions] = useState<number[]>([]);
  const [saving, setSaving] = useState(false);
  const [appVersion, setAppVersion] = useState<string | null>(null);
  const [updateState, setUpdateState] = useState<UpdateState>({ kind: "idle" });

  useEffect(() => {
    invoke<Settings>("get_settings").then((value) => {
      const theme = value.theme ?? "classic";
      const appearance = value.appearance ?? "system";
      applyTheme(theme, appearance);
      setSettings({ ...value, theme, appearance });
    }).catch(console.error);
    invoke<number[]>("interval_options").then(setOptions).catch(console.error);
    getVersion().then(setAppVersion).catch(console.error);
    return subscribeUpdate(setUpdateState);
  }, []);

  const update = async (patch: Partial<Settings>) => {
    if (!settings) return;
    const previous = settings;
    const next = { ...settings, ...patch };
    if (patch.theme || patch.appearance) {
      applyTheme(next.theme, next.appearance);
    }
    setSettings(next); // 乐观更新
    setSaving(true);
    try {
      const saved = await invoke<Settings>("set_general_settings", {
        launchAtLogin: next.launch_at_login,
        refreshIntervalSecs: next.refresh_interval_secs,
        theme: next.theme,
        appearance: next.appearance,
        floatingOrbEnabled: next.floating_orb.enabled,
      });
      setSettings(saved);
    } catch (e) {
      console.error("保存设置失败", e);
      applyTheme(previous.theme, previous.appearance);
      setSettings(previous);
    } finally {
      setSaving(false);
    }
  };

  if (!settings) return null;

  const updateLabel = (() => {
    switch (updateState.kind) {
      case "idle": return "检查更新";
      case "checking": return "检查中…";
      case "none": return "已是最新版本";
      case "available": return `下载 v${updateState.version}`;
      case "downloading": return `下载中 ${updateState.percent}%`;
      case "ready": return "升级并重启";
      case "installing": return "正在升级…";
      case "install_error": return "重试升级";
      case "error": return "重试检查更新";
    }
  })();

  const onUpdateClick = async () => {
    switch (updateState.kind) {
      case "available":
        await download();
        break;
      case "ready":
      case "install_error":
        await installAndRelaunch();
        break;
      default:
        await checkForUpdate();
    }
  };

  const busy = updateState.kind === "checking"
    || updateState.kind === "downloading"
    || updateState.kind === "installing";

  return (
    <div className="settings">
      <label className="settings-row">
        <input
          type="checkbox"
          checked={settings.launch_at_login}
          onChange={(e) => update({ launch_at_login: e.target.checked })}
          disabled={saving}
        />
        <span>开机自动启动</span>
      </label>

      <label className="settings-row">
        <input
          type="checkbox"
          checked={settings.floating_orb.enabled}
          onChange={(event) => update({
            floating_orb: { ...settings.floating_orb, enabled: event.target.checked },
          })}
          disabled={saving}
        />
        <span>桌面悬浮球</span>
      </label>

      <div className="settings-theme-row">
        <span className="settings-label">界面主题</span>
        <span className="theme-switch" role="group" aria-label="界面主题">
          <button
            type="button"
            className={settings.theme === "classic" ? "active" : ""}
            onClick={() => update({ theme: "classic" })}
            disabled={saving}
          >
            Classic
          </button>
          <button
            type="button"
            className={settings.theme === "parchment" ? "active" : ""}
            onClick={() => update({ theme: "parchment" })}
            disabled={saving}
          >
            Claude
          </button>
          <button
            type="button"
            className={settings.theme === "cyberpunk" ? "active" : ""}
            onClick={() => update({ theme: "cyberpunk" })}
            disabled={saving}
          >
            Cyber
          </button>
        </span>
      </div>

      <div className="settings-theme-row appearance-row">
        <span className="settings-label">明暗外观</span>
        <span className="appearance-switch" role="group" aria-label="明暗外观">
          {(["system", "light", "dark"] as const).map((appearance) => (
            <button
              key={appearance}
              type="button"
              className={settings.appearance === appearance ? "active" : ""}
              onClick={() => update({ appearance })}
              disabled={saving}
            >
              {appearance === "system" ? "系统" : appearance === "light" ? "浅色" : "深色"}
            </button>
          ))}
        </span>
      </div>

      <label className="settings-row">
        <span className="settings-label">后台刷新间隔</span>
        <select
          value={settings.refresh_interval_secs}
          onChange={(e) => update({ refresh_interval_secs: Number(e.target.value) })}
          disabled={saving}
        >
          {options.map((s) => (
            <option key={s} value={s}>
              {INTERVAL_LABELS[s] ?? `${s} 秒`}
            </option>
          ))}
        </select>
      </label>

      <div className="settings-update-row">
        <div className="settings-version">
          <span>当前版本</span>
          <strong className="tnum">{appVersion ? `v${appVersion}` : "—"}</strong>
        </div>
        <button
          className={`settings-update-btn ${updateState.kind === "available" || updateState.kind === "ready" ? "has-update" : ""}`}
          onClick={onUpdateClick}
          disabled={busy}
        >
          {updateLabel}
        </button>
      </div>

      <div className="settings-hint">打开面板时会立即刷新</div>

      <button
        className="settings-quit-btn"
        onClick={() => invoke("quit_app").catch(console.error)}
        title="退出 TokenMeter"
      >
        退出应用
      </button>
    </div>
  );
}
