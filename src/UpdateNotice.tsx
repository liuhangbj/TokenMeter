import type { UpdateState } from "./updater";

export function UpdateNotice({
  state,
  onInstall,
}: {
  state: UpdateState;
  onInstall: () => void;
}) {
  if (state.kind !== "ready" && state.kind !== "installing" && state.kind !== "install_error") {
    return null;
  }

  const installing = state.kind === "installing";
  const failed = state.kind === "install_error";
  return (
    <div className="update-notice" role="status" aria-live="polite">
      <span className="update-notice-mark" aria-hidden="true">↑</span>
      <span className="update-notice-copy">
        <strong>{failed ? `v${state.version} 安装未完成` : `新版本 v${state.version} 已准备好`}</strong>
        <span>{failed ? "可以继续使用，稍后重试升级" : "由你决定何时安装，不会打断当前使用"}</span>
      </span>
      <button type="button" onClick={onInstall} disabled={installing} title={failed ? state.message : undefined}>
        {installing ? "正在升级…" : failed ? "重试升级" : "升级并重启"}
      </button>
    </div>
  );
}
