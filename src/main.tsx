import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { invoke } from "@tauri-apps/api/core";
import "./theme.css";
import "./popover.css";
import "./wizard.css";

// 全局 JS 错误兜底：任何未捕获错误直接把原文显示在页面上（排查空白页），
// 并同步上报到后端日志（TOKENMETER_LOG_FILE）。
function showFatalError(msg: string) {
  const el = document.getElementById("root");
  if (el) {
    el.innerHTML =
      `<div style="font:12px/1.6 monospace;color:#d33;background:#fff;padding:20px;white-space:pre-wrap;word-break:break-all">` +
      msg.replace(/&/g, "&amp;").replace(/</g, "&lt;") +
      `</div>`;
  }
  invoke("log_frontend_error", { msg }).catch(() => {});
}
window.addEventListener("error", (e) => {
  showFatalError(`[error] ${e.message}\n${e.filename}:${e.lineno}`);
});
window.addEventListener("unhandledrejection", (e) => {
  showFatalError(`[unhandledrejection] ${String(e.reason)}`);
});

function Root() {
  return <App />;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>
);
