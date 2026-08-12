// 自动更新封装：后台定时检查并下载，始终由用户确认后才安装重启。
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "none" }                       // 已是最新
  | { kind: "available"; version: string } // 有新版本
  | { kind: "downloading"; version: string; percent: number }
  | { kind: "ready"; version: string }      // 下载完成，等待用户确认
  | { kind: "installing"; version: string }
  | { kind: "install_error"; version: string; message: string }
  | { kind: "error"; message: string };

const LAST_CHECK_KEY = "tm_last_update_check";
const READY_VERSION_KEY = "tm_ready_update_version";
const AUTO_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
const RETRY_DELAY_MS = 15 * 60 * 1000;

// ---- 模块级共享状态 + 订阅（启动自动检查的进度要被设置区按钮感知）----
let current: UpdateState = { kind: "idle" };
let pending: Update | null = null;
let activeOperation: Promise<boolean> | null = null;
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

function readyVersion(): string | null {
  return localStorage.getItem(READY_VERSION_KEY)?.trim() || null;
}

function lastCheckAt(): number {
  return Number(localStorage.getItem(LAST_CHECK_KEY) ?? 0);
}

function markCheckedNow() {
  localStorage.setItem(LAST_CHECK_KEY, String(Date.now()));
}

function shouldCheckNow(): boolean {
  // 上次已下载但应用在安装前退出：必须重新建立 Update 对象并重新下载，
  // 不能被普通检查间隔拦住。
  if (readyVersion()) return pending === null;
  return Date.now() - lastCheckAt() >= AUTO_CHECK_INTERVAL_MS;
}

function hasPendingInstallChoice(): boolean {
  return pending !== null
    && (current.kind === "ready" || current.kind === "installing" || current.kind === "install_error");
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
      pending = null;
      localStorage.removeItem(READY_VERSION_KEY);
      setState({ kind: "none" });
    }
  } catch (e) {
    // 开发模式下 updater 不可用（无签名产物），静默降级
    console.warn("update check failed:", e);
    setState({ kind: "error", message: String(e) });
  }
}

function onDownloadEvent(event: DownloadEvent) {
  const version = pending?.version ?? "";
  if (event.event === "Started") {
    downloadTotal = event.data.contentLength ?? 0;
    downloadedBytes = 0;
    setState({ kind: "downloading", version, percent: 0 });
  } else if (event.event === "Progress") {
    downloadedBytes += event.data.chunkLength;
    const percent = downloadTotal > 0 ? Math.round((downloadedBytes / downloadTotal) * 100) : 0;
    setState({ kind: "downloading", version, percent });
  }
}

let downloadedBytes = 0;
let downloadTotal = 0;

/// 只下载新版，不安装、不退出应用。Windows 与 macOS 都在用户确认后才安装。
export async function download(): Promise<boolean> {
  if (!pending) return false;
  const version = pending.version;
  try {
    await pending.download(onDownloadEvent);
    localStorage.setItem(READY_VERSION_KEY, version);
    setState({ kind: "ready", version });
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
  const version = pending.version;
  setState({ kind: "installing", version });
  // Windows 安装器可能在 install() 内直接结束当前进程，所以安装前清除标记；
  // 若安装失败，catch 会恢复标记供下次重试。
  localStorage.removeItem(READY_VERSION_KEY);
  try {
    await pending.install();
    await relaunch();
  } catch (e) {
    localStorage.setItem(READY_VERSION_KEY, version);
    setState({ kind: "install_error", version, message: String(e) });
  }
}

/// 自动检查：有新版则后台下载到 ready；绝不自动安装或退出应用。
export async function autoCheckOnLaunch(): Promise<boolean> {
  if (activeOperation) return activeOperation;
  activeOperation = (async () => {
  try {
    const update = await check();
    if (update) {
      pending = update;
      setState({ kind: "available", version: update.version });
      if (!await download()) return false;
    } else {
      pending = null;
      localStorage.removeItem(READY_VERSION_KEY);
      setState({ kind: "none" });
    }
    markCheckedNow();
    return true;
  } catch (e) {
    console.warn("auto update check failed:", e);
    setState({ kind: "error", message: String(e) });
    return false;
  } finally {
    activeOperation = null;
  }
  })();
  return activeOperation;
}

/**
 * 应用生命周期内持续监控更新。启动后立即按需检查，之后每 6 小时检查；
 * 网络恢复时也会补检。返回清理函数供 React effect 卸载。
 */
export function startUpdateMonitoring(): () => void {
  let stopped = false;
  let retryTimer: number | null = null;

  const run = async (force = false) => {
    // 更新已准备好后维持用户的安装选择，不在后台重复下载或覆盖提示状态。
    if (stopped || hasPendingInstallChoice() || (!force && !shouldCheckNow())) return;
    const success = await autoCheckOnLaunch();
    if (success && retryTimer !== null) {
      window.clearTimeout(retryTimer);
      retryTimer = null;
    } else if (!success && !stopped && retryTimer === null) {
      retryTimer = window.setTimeout(() => {
        retryTimer = null;
        void run(true);
      }, RETRY_DELAY_MS);
    }
  };

  // 每次应用进程启动都立即检查一次；后续由间隔与联网恢复事件接管。
  void run(true);
  const interval = window.setInterval(() => void run(), AUTO_CHECK_INTERVAL_MS);
  const onOnline = () => void run(true);
  window.addEventListener("online", onOnline);

  return () => {
    stopped = true;
    window.clearInterval(interval);
    if (retryTimer !== null) window.clearTimeout(retryTimer);
    window.removeEventListener("online", onOnline);
  };
}
