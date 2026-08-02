// 自动更新封装：启动静默检查 + 自动下载 + 一键重启
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "none" }                       // 已是最新
  | { kind: "available"; version: string } // 有新版本
  | { kind: "downloading"; percent: number }
  | { kind: "ready" }                      // 下载完成，待重启
  | { kind: "error"; message: string };

// ---- 模块级共享状态 + 订阅（启动自动检查的进度要被设置区按钮感知）----
let current: UpdateState = { kind: "idle" };
let pending: Update | null = null;
const listeners = new Set<(s: UpdateState) => void>();

function setState(s: UpdateState) {
  current = s;
  listeners.forEach((fn) => fn(s));
}
/** 订阅状态变化，返回取消函数；立即同步当前状态 */
export function subscribeUpdate(fn: (s: UpdateState) => void): () => void {
  listeners.add(fn);
  fn(current);
  return () => listeners.delete(fn);
}
export function currentUpdateState(): UpdateState {
  return current;
}

/// 检查更新（静默，不抛错）
export async function checkForUpdate(): Promise<void> {
  setState({ kind: "checking" });
  try {
    const update = await check();
    if (update) {
      pending = update;
      setState({ kind: "available", version: update.version });
    } else {
      setState({ kind: "none" });
    }
  } catch (e) {
    // 开发模式下 updater 不可用（无签名产物），静默降级
    console.warn("update check failed:", e);
    setState({ kind: "error", message: String(e) });
  }
}

/// 下载新版（不重启）
export async function download(): Promise<void> {
  if (!pending) return;
  try {
    let downloaded = 0;
    let total = 0;
    await pending.downloadAndInstall((event) => {
      if (event.event === "Started") {
        total = event.data.contentLength ?? 0;
        setState({ kind: "downloading", percent: 0 });
      } else if (event.event === "Progress") {
        downloaded += event.data.chunkLength;
        const percent = total > 0 ? Math.round((downloaded / total) * 100) : 0;
        setState({ kind: "downloading", percent });
      } else if (event.event === "Finished") {
        setState({ kind: "ready" });
      }
    });
  } catch (e) {
    setState({ kind: "error", message: String(e) });
  }
}

/// 重启并完成安装（用户确认后调用）
export async function relaunchToInstall(): Promise<void> {
  await relaunch();
}

/// 启动时自动检查：有新版 → 后台自动下载到 ready（不打扰；重启由用户确认）
export async function autoCheckOnLaunch(): Promise<void> {
  try {
    const update = await check();
    if (update) {
      pending = update;
      setState({ kind: "available", version: update.version });
      await download();
    } else {
      setState({ kind: "none" });
    }
  } catch (e) {
    console.warn("auto update check failed:", e);
  }
}
