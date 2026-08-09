// 添加供应商：厂商折叠分组 → 产品类型 → 数据驱动的认证流程。
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-shell";
import type { AddableProvider, AuthField, AuthSpec } from "./types";
import { brandStyleVars } from "./utils";
import {
  BrandIcon,
  IconApi,
  IconArrowLeft,
  IconChevronDown,
  IconPlan,
} from "./icons";

type Step =
  | { kind: "pick" }
  | { kind: "form"; provider: AddableProvider }
  | { kind: "oauth"; provider: AddableProvider };

interface VendorGroup {
  vendor: AddableProvider["vendor"];
  products: AddableProvider[];
}

function primaryAuthKind(spec: AuthSpec): AuthSpec["kind"] {
  return spec.kind === "hybrid" ? primaryAuthKind(spec.primary) : spec.kind;
}

function authFields(spec: AuthSpec): AuthField[] {
  if (spec.kind === "api_key" || spec.kind === "cloud_secret") return spec.fields;
  if (spec.kind === "hybrid") return authFields(spec.primary);
  return [];
}

function authHint(spec: AuthSpec): string | null {
  if (spec.kind === "api_key") return spec.hint;
  if (spec.kind === "hybrid") return authHint(spec.primary);
  return null;
}

function WizardHeader({
  title,
  subtitle,
  onBack,
}: {
  title: string;
  subtitle: string;
  onBack: () => void;
}) {
  return (
    <div className="wizard-header">
      <button className="wizard-back" type="button" onClick={onBack} aria-label="返回">
        <IconArrowLeft />
      </button>
      <div className="wizard-heading">
        <div className="wizard-title">{title}</div>
        <div className="wizard-sub">{subtitle}</div>
      </div>
      <span className="wizard-head-balance" aria-hidden="true" />
    </div>
  );
}

function ProductIntro({ provider }: { provider: AddableProvider }) {
  const isPlan = provider.account_type === "plan";
  return (
    <div
      className="auth-product"
      data-brand={provider.vendor.brand.key}
      style={brandStyleVars(provider.vendor.brand)}
    >
      <span className="auth-product-brand">
        <BrandIcon
          brand={provider.vendor.brand.key}
          label={provider.vendor.display_name}
          size={20}
        />
      </span>
      <span className="auth-product-copy">
        <span className="auth-product-vendor">{provider.vendor.display_name}</span>
        <span className="auth-product-name">{provider.product_name}</span>
      </span>
      <span className={`account-type-chip ${provider.account_type}`}>
        {isPlan ? "PLAN" : "API"}
      </span>
    </div>
  );
}

export function AddProvider({ onDone }: { onDone: () => void }) {
  const [providers, setProviders] = useState<AddableProvider[]>([]);
  const [step, setStep] = useState<Step>({ kind: "pick" });
  const [expandedVendor, setExpandedVendor] = useState<string | null>(null);
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

  const groups = useMemo<VendorGroup[]>(() => {
    const grouped = new Map<string, VendorGroup>();
    for (const provider of providers) {
      const existing = grouped.get(provider.vendor.id);
      if (existing) existing.products.push(provider);
      else grouped.set(provider.vendor.id, { vendor: provider.vendor, products: [provider] });
    }
    return [...grouped.values()];
  }, [providers]);

  const pick = (provider: AddableProvider) => {
    setError(null);
    setBusy(false);
    setSuccess(false);
    if (primaryAuthKind(provider.auth_spec) === "oauth") {
      setStep({ kind: "oauth", provider });
    } else {
      setStep({ kind: "form", provider });
    }
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
            setSuccess(true);
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
    <div className="wizard wizard-picker">
      <WizardHeader title="添加供应商" subtitle="选择厂商与账户类型" onBack={onDone} />
      <div className="wizard-main">
        {loadError && <div className="form-status error">加载供应商列表失败：{loadError}</div>}
        {!loadError && providers.length === 0 && (
          <div className="wizard-empty">正在加载供应商…</div>
        )}

        <div className="vendor-list">
          {groups.map((group) => {
            const multiple = group.products.length > 1;
            const expanded = multiple && expandedVendor === group.vendor.id;
            const directProduct = multiple ? null : group.products[0];
            return (
              <section
                key={group.vendor.id}
                className={`vendor-group ${expanded ? "expanded" : ""}`}
                data-brand={group.vendor.brand.key}
                style={brandStyleVars(group.vendor.brand)}
              >
                <button
                  className="vendor-trigger"
                  type="button"
                  aria-expanded={multiple ? expanded : undefined}
                  onClick={() => {
                    if (directProduct) pick(directProduct);
                    else setExpandedVendor(expanded ? null : group.vendor.id);
                  }}
                >
                  <span className="vendor-brand">
                    <BrandIcon
                      brand={group.vendor.brand.key}
                      label={group.vendor.display_name}
                      size={22}
                    />
                  </span>
                  <span className="vendor-name">{group.vendor.display_name}</span>
                  <span className="vendor-count">
                    {multiple ? `${group.products.length} 种账户` : group.products[0].product_name}
                  </span>
                  <span className={`vendor-chevron ${multiple ? "" : "direct"} ${expanded ? "open" : ""}`}>
                    <IconChevronDown />
                  </span>
                </button>

                {expanded && (
                  <div className="provider-options">
                    {group.products.map((provider) => (
                      <button
                        key={provider.id}
                        className={`provider-option ${provider.account_type}`}
                        type="button"
                        onClick={() => pick(provider)}
                      >
                        <span className="provider-type-icon">
                          {provider.account_type === "plan" ? <IconPlan /> : <IconApi />}
                        </span>
                        <span className="provider-option-copy">
                          <span className="provider-option-title">{provider.product_name}</span>
                          <span className="provider-option-desc">{provider.description}</span>
                        </span>
                        <span className={`account-type-chip ${provider.account_type}`}>
                          {provider.account_type === "plan" ? "PLAN" : "API"}
                        </span>
                        <span className="provider-option-chevron"><IconChevronDown /></span>
                      </button>
                    ))}
                  </div>
                )}
              </section>
            );
          })}
        </div>

        {providers.length > 0 && (
          <div className="wizard-note"><span>ⓘ</span> 同一厂商可添加多个账户</div>
        )}
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
  const fields = authFields(provider.auth_spec);
  const hint = authHint(provider.auth_spec);
  const [values, setValues] = useState<Record<string, string>>({});

  return (
    <div className="wizard wizard-auth">
      <WizardHeader title="添加账户" subtitle="填写凭证并验证连接" onBack={onBack} />
      <div className="wizard-main">
        <ProductIntro provider={provider} />
        {hint && <div className="auth-hint">{hint}</div>}
        <div className="form auth-form">
          {fields.map((field) => (
            <label key={field.key} className="form-field">
              <span className="form-label">
                {field.label}{field.required && <em>*</em>}
              </span>
              {field.options ? (
                <select
                  value={values[field.key] ?? ""}
                  onChange={(event) => setValues({ ...values, [field.key]: event.target.value })}
                >
                  <option value="">请选择</option>
                  {field.options.map(([value, label]) => (
                    <option key={value} value={value}>{label}</option>
                  ))}
                </select>
              ) : (
                <input
                  type={field.secret ? "password" : "text"}
                  placeholder={field.placeholder}
                  value={values[field.key] ?? ""}
                  onChange={(event) => setValues({ ...values, [field.key]: event.target.value })}
                />
              )}
            </label>
          ))}
        </div>
        {error && <div className="form-status error">{error}</div>}
        {success && <div className="form-status success">验证成功，凭证已保存</div>}
        <button
          className="btn primary block auth-submit"
          type="button"
          disabled={busy || success}
          onClick={() => onSubmit(values)}
        >
          {success ? "已保存" : busy ? "正在验证…" : "保存并验证"}
        </button>
      </div>
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
  type Status =
    | { kind: "idle" }
    | { kind: "working"; note: string }
    | { kind: "device"; code: string }
    | { kind: "success" }
    | { kind: "error"; msg: string };
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const busy = status.kind === "working" || status.kind === "device";

  const tryImport = async () => {
    setStatus({ kind: "working", note: "正在检测本机凭证…" });
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

  const startDevice = async () => {
    setStatus({ kind: "working", note: "正在请求授权码…" });
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

  const startCodex = async () => {
    setStatus({ kind: "working", note: "正在请求授权码…" });
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
    <div className="wizard wizard-auth">
      <WizardHeader title="添加账户" subtitle="授权登录并同步账户信息" onBack={onBack} />
      <div className="wizard-main">
        <ProductIntro provider={provider} />
        <div className="oauth-copy">
          可通过浏览器完成安全授权，也可以导入本机已经登录的 CLI 凭证。
        </div>

        {status.kind === "device" && (
          <div className="form-status working device-status">
            <span>浏览器授权码</span>
            <strong>{status.code}</strong>
            <small>完成登录后，此窗口会自动继续</small>
          </div>
        )}
        {status.kind === "working" && <div className="form-status working">{status.note}</div>}
        {status.kind === "success" && <div className="form-status success">账户已添加，凭证已保存</div>}
        {status.kind === "error" && <div className="form-status error">{status.msg}</div>}

        <div className="oauth-actions">
          {(isKimi || isCodex) && (
            <button
              className="btn primary block"
              type="button"
              disabled={busy || status.kind === "success"}
              onClick={isKimi ? startDevice : startCodex}
            >
              {busy ? "等待浏览器授权…" : "浏览器授权登录"}
            </button>
          )}
          <button
            className="btn block secondary"
            type="button"
            disabled={busy || status.kind === "success"}
            onClick={tryImport}
          >
            导入本机 CLI 凭证
          </button>
        </div>
      </div>
    </div>
  );
}
