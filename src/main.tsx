import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { AddProvider } from "./AddProvider";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./theme.css";
import "./popover.css";
import "./wizard.css";

// 根据【窗口 label】决定渲染哪个界面（跨平台最稳，不依赖 URL query）：
// - add-provider 窗口 → 添加供应商向导（独立窗口）
// - popover / 默认     → 托盘下拉面板
const label = getCurrentWindow().label;
const isAddWizard = label === "add-provider";

function Root() {
  if (isAddWizard) {
    return <AddProvider onDone={() => window.close()} />;
  }
  return <App />;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>
);
