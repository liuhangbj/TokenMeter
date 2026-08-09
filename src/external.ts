import { invoke } from "@tauri-apps/api/core";

/** 统一交给后端调用系统默认浏览器，确保 Windows 纯托盘模式也能打开。 */
export async function openExternal(url: string): Promise<void> {
  await invoke("open_external", { url });
}
