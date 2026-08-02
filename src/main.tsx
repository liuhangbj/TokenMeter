import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { AddProvider } from "./AddProvider";
import "./theme.css";
import "./popover.css";
import "./wizard.css";

// 根据 URL 参数决定渲染哪个界面：
// - index.html?add=1 → 添加供应商向导（独立窗口）
// - 默认             → 托盘下拉面板
const isAddWizard = new URLSearchParams(window.location.search).has("add");

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
