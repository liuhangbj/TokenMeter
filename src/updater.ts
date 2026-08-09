// 自动更新封装：启动静默检查 + 仅下载 + 用户确认后安装重启
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
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

function onDownloadEvent(event: DownloadEvent) {
  if (event.event === "Started") {
    downloadTotal = event.data.contentLength ?? 0;
    downloadedBytes = 0;
    setState({ kind: "downloading", percent: 0 });
  } else if (event.event === "Progress") {
    downloadedBytes += event.data.chunkLength;
    const percent = downloadTotal > 0 ? Math.round((downloadedBytes / downloadTotal) * 100) : 0;
    setState({ kind: "downloading", percent });
  }
}

let downloadedBytes = 0;
let downloadTotal = 0;

/// 只下载新版，不安装、不退出应用。Windows 与 macOS 都在用户确认后才安装。
export async function download(): Promise<boolean> {
  if (!pending) return false;
  try {
    await pending.download(onDownloadEvent);
    setState({ kind: "ready" });
    return true;
  } catch (e) {
    setState({ kind: "error", message: String(e) });
    return false;
  }
}

/// 用户确认后安装并重启。Windows 安装器可能在 install() 内主动结束进程；
/// 若 install() 正常返回（macOS 等），再显式重启加载新版本。
export async function installAndRelaunch(): Promise<void> {
  if (!pending) return;
  try {
    await pending.install();
    await relaunch();
  } catch (e) {
    setState({ kind: "error", message: String(e) });
  }
}

/// 启动时自动检查：有新版 → 后台自动下载到 ready（不打扰；重启由用户确认）
export async function autoCheckOnLaunch(): Promise<boolean> {
  try {
    const update = await check();
    if (update) {
      pending = update;
      setState({ kind: "available", version: update.version });
      if (!await download()) return false;
    } else {
      setState({ kind: "none" });
    }
    return true;
  } catch (e) {
    console.warn("auto update check failed:", e);
    setState({ kind: "error", message: String(e) });
    return false;
  }
}
