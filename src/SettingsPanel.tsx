// 设置区：开机启动勾选 + 后台刷新间隔下拉 + 检查更新
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  subscribeUpdate,
  checkForUpdate,
  download,
  relaunchToInstall,
  type UpdateState,
} from "./updater";

interface Settings {
  launch_at_login: boolean;
  refresh_interval_secs: number;
}

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
  const [updateState, setUpdateState] = useState<UpdateState>({ kind: "idle" });

  useEffect(() => {
    invoke<Settings>("get_settings").then(setSettings).catch(console.error);
    invoke<number[]>("interval_options").then(setOptions).catch(console.error);
    return subscribeUpdate(setUpdateState);
  }, []);

  const update = async (patch: Partial<Settings>) => {
    if (!settings) return;
    const next = { ...settings, ...patch };
    setSettings(next); // 乐观更新
    setSaving(true);
    try {
      await invoke("set_settings", { settings: next });
    } catch (e) {
      console.error("保存设置失败", e);
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
      case "ready": return "重启完成更新";
      case "error": return "重试检查更新";
    }
  })();

  const onUpdateClick = async () => {
    switch (updateState.kind) {
      case "available":
        await download();
        break;
      case "ready":
        await relaunchToInstall();
        break;
      default:
        await checkForUpdate();
    }
  };

  const busy = updateState.kind === "checking" || updateState.kind === "downloading";

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

      <button
        className={`settings-update-btn ${updateState.kind === "available" || updateState.kind === "ready" ? "has-update" : ""}`}
        onClick={onUpdateClick}
        disabled={busy}
      >
        {updateLabel}
      </button>

      <div className="settings-hint">打开面板时会立即刷新</div>
    </div>
  );
}
