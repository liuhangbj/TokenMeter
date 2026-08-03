// 添加供应商向导：网格选择 → 动态表单（API Key）/ OAuth 授权 / 本机导入
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-shell";
import type { AddableProvider, AuthField } from "./types";
import { brandOf } from "./utils";

type Step =
  | { kind: "pick" }
  | { kind: "form"; provider: AddableProvider }
  | { kind: "oauth"; provider: AddableProvider };

const KIND_LABEL: Record<string, string> = {
  oauth: "浏览器授权",
  api_key: "API Key",
  cloud_secret: "SecretId/Key",
  hybrid: "混合",
};

export function AddProvider({ onDone }: { onDone: () => void }) {
  const [providers, setProviders] = useState<AddableProvider[]>([]);
  const [step, setStep] = useState<Step>({ kind: "pick" });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [success, setSuccess] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    invoke<AddableProvider[]>("list_addable_providers")
      .then((list) => setProviders(list))
      .catch((e) => {
        console.error("list_addable_providers 失败", e);
        setLoadError(String(e));
      });
  }, []);

  const pick = (p: AddableProvider) => {
    setError(null);
    if (p.auth_spec.kind === "oauth") setStep({ kind: "oauth", provider: p });
    else setStep({ kind: "form", provider: p });
  };

  if (step.kind === "form") {
    return (
      <ApiKeyForm
        provider={step.provider}
        busy={busy}
        error={error}
        success={success}
        onBack={() => setStep({ kind: "pick" })}
        onSubmit={async (values) => {
          setBusy(true);
          setError(null);
          setSuccess(false);
          try {
            await invoke("save_api_key_provider", {
              providerId: step.provider.id,
              fields: values,
            });
            setSuccess(true); // 显示"✅ 验证成功"，延迟关窗
            setTimeout(onDone, 1200);
          } catch (e) {
            setError(String(e));
            setBusy(false);
          }
        }}
      />
    );
  }

  if (step.kind === "oauth") {
    return (
      <OAuthFlow
        provider={step.provider}
        onBack={() => setStep({ kind: "pick" })}
        onDone={onDone}
      />
    );
  }

  return (
    <div className="wizard">
      <button className="link-back" onClick={onDone}>← 返回</button>
      <div className="wizard-title">添加供应商</div>
      <div className="wizard-sub">选择要监控的平台</div>
      {loadError && <div className="form-error">加载供应商列表失败：{loadError}</div>}
      {!loadError && providers.length === 0 && (
        <div className="empty">暂无可用供应商（加载中或列表为空）</div>
      )}
      <div className="provider-grid">
        {providers.map((p) => (
          <button key={p.id} className="provider-cell" data-brand={brandOf(p.id)} onClick={() => pick(p)}>
            <span className="provider-dot" />
            <span className="provider-name">{p.display_name}</span>
            <span className="provider-kind">{KIND_LABEL[p.auth_spec.kind] ?? p.auth_spec.kind}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function ApiKeyForm({
  provider, busy, error, success, onBack, onSubmit,
}: {
  provider: AddableProvider;
  busy: boolean;
  error: string | null;
  success: boolean;
  onBack: () => void;
  onSubmit: (values: Record<string, string>) => void;
}) {
  const fields: AuthField[] =
    provider.auth_spec.kind === "api_key" || provider.auth_spec.kind === "cloud_secret"
      ? provider.auth_spec.fields
      : [];
  const hint = provider.auth_spec.kind === "api_key" ? provider.auth_spec.hint : null;
  const [values, setValues] = useState<Record<string, string>>({});

  return (
    <div className="wizard">
      <button className="link-back" onClick={onBack}>← 返回</button>
      <div className="wizard-title">{provider.display_name}</div>
      {hint && <div className="wizard-sub">{hint}</div>}
      <div className="form">
        {fields.map((f) => (
          <label key={f.key} className="form-field">
            <span className="form-label">
              {f.label}{f.required && <em>*</em>}
            </span>
            {f.options ? (
              <select
                value={values[f.key] ?? ""}
                onChange={(e) => setValues({ ...values, [f.key]: e.target.value })}
              >
                <option value="">请选择</option>
                {f.options.map(([v, label]) => (
                  <option key={v} value={v}>{label}</option>
                ))}
              </select>
            ) : (
              <input
                type={f.secret ? "password" : "text"}
                placeholder={f.placeholder}
                value={values[f.key] ?? ""}
                onChange={(e) => setValues({ ...values, [f.key]: e.target.value })}
              />
            )}
          </label>
        ))}
      </div>
      {error && <div className="form-error">{error}</div>}
      {success && <div className="form-ok">✅ 验证成功，已保存凭证</div>}
      <button className="btn primary block" disabled={busy || success} onClick={() => onSubmit(values)}>
        {success ? "已保存" : busy ? "验证中…" : "保存并验证"}
      </button>
    </div>
  );
}

function OAuthFlow({
  provider, onBack, onDone,
}: {
  provider: AddableProvider;
  onBack: () => void;
  onDone: () => void;
}) {
  // 统一状态机，杜绝"已导入"和"等待授权"叠加（用户反馈两提示同时出现）
  type Status =
    | { kind: "idle" }
    | { kind: "working"; note: string }   // 导入中 / 等待浏览器授权
    | { kind: "device"; code: string }    // Kimi 设备码授权中
    | { kind: "success" }
    | { kind: "error"; msg: string };
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  const busy = status.kind === "working" || status.kind === "device";

  // Codex/Kimi 本机已装 CLI 时可一键导入
  const tryImport = async () => {
    setStatus({ kind: "working", note: "检测本机凭证…" });
    try {
      const ok = await invoke<boolean>("import_local_credential", { providerId: provider.id });
      if (ok) {
        setStatus({ kind: "success" });
        setTimeout(onDone, 800);
      } else {
        setStatus({ kind: "error", msg: "未检测到本机已登录的 CLI 凭证，请改用浏览器授权" });
      }
    } catch (e) {
      setStatus({ kind: "error", msg: String(e) });
    }
  };

  // Kimi 设备码流程
  const startDevice = async () => {
    setStatus({ kind: "working", note: "请求授权码…" });
    try {
      const start = await invoke<{ user_code: string; verify_url: string; device_code: string; interval_secs: number }>(
        "kimi_device_start"
      );
      setStatus({ kind: "device", code: start.user_code });
      await open(start.verify_url);
      await invoke("kimi_device_poll", {
        deviceCode: start.device_code,
        intervalSecs: start.interval_secs,
      });
      setStatus({ kind: "success" });
      setTimeout(onDone, 800);
    } catch (e) {
      setStatus({ kind: "error", msg: String(e) });
    }
  };

  // Codex 设备码授权（官方新版流程）：与 Kimi 同款，先拿设备码再轮询
  const startCodex = async () => {
    setStatus({ kind: "working", note: "请求授权码…" });
    try {
      const start = await invoke<{ user_code: string; verify_url: string; device_auth_id: string; interval_secs: number }>(
        "codex_device_start"
      );
      setStatus({ kind: "device", code: start.user_code });
      await open(start.verify_url);
      await invoke("codex_device_poll", {
        deviceAuthId: start.device_auth_id,
        userCode: start.user_code,
        intervalSecs: start.interval_secs,
      });
      setStatus({ kind: "success" });
      setTimeout(onDone, 800);
    } catch (e) {
      setStatus({ kind: "error", msg: String(e) });
    }
  };

  const isKimi = provider.id === "kimi_code";
  const isCodex = provider.id === "codex";

  return (
    <div className="wizard">
      <button className="link-back" onClick={onBack}>← 返回</button>
      <div className="wizard-title">{provider.display_name}</div>
      <div className="wizard-sub">通过浏览器登录授权，或导入本机已登录的 CLI 凭证</div>

      {/* 单一状态提示区：任何时刻只显示一条 */}
      {status.kind === "device" && (
        <div className="device-code">
          授权码 <strong>{status.code}</strong> 已在浏览器打开，完成登录后此窗自动继续…
        </div>
      )}
      {status.kind === "working" && (
        <div className="device-code">{status.note}</div>
      )}
      {status.kind === "success" && <div className="form-ok">✅ 已添加，凭证已保存</div>}
      {status.kind === "error" && <div className="form-error">{status.msg}</div>}

      <div className="oauth-actions">
        <button className="btn block" disabled={busy} onClick={tryImport}>
          导入本机 CLI 凭证
        </button>
        {isKimi && (
          <button className="btn primary block" disabled={busy} onClick={startDevice}>
            {status.kind === "device" ? "等待授权…" : "浏览器授权登录"}
          </button>
        )}
        {isCodex && (
          <button className="btn primary block" disabled={busy} onClick={startCodex}>
            {busy ? "等待授权…" : "浏览器授权登录"}
          </button>
        )}
      </div>
    </div>
  );
}
