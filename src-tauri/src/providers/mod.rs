//! Provider 抽象层
//!
//! 把 8 个平台的差异收敛到统一的 `Provider` trait 与数据模型，
//! UI 层与调度器不感知具体平台。新增平台只需实现 trait，
//! 「添加供应商」表单由 `auth_spec()` 数据驱动自动渲染。
#![allow(dead_code)] // Provider API surface; consumed by M2/M3 add-provider UI, not yet read in M1

pub mod codex;
pub mod deepseek;
pub mod kimi_code;
pub mod moonshot;
pub mod openai_platform;
pub mod tencent;
pub mod tencent_coding_plan;
pub mod tencent_token_plan;
pub mod tencent_tokenhub;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ---------- 统一数据模型 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BillingMode {
    /// 订阅：有套餐名 + 周期窗口
    Subscription,
    /// 按量：有账户余额 + 消耗统计
    PayAsYouGo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QuotaUnit {
    Percent,
    Requests,
    Tokens,
    Currency(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowPeriod {
    Hours5,
    Day,
    Week,
    Month,
    Custom(i64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub period: WindowPeriod,
    pub label: String,
    pub used: Option<f64>,
    pub limit: Option<f64>,
    pub remaining: Option<f64>,
    pub unit: QuotaUnit,
    pub reset_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    pub total: f64,
    pub granted: Option<f64>,
    pub topped_up: Option<f64>,
    pub currency: String,
    pub available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fidelity {
    /// 官方接口直接返回
    Exact,
    /// 非官方实时来源（本地记账 / 逆向接口，M5 增强用）
    Estimated,
    /// 部分维度缺失
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Ok,
    AuthExpired,
    Degraded,
    Exhausted,
    NetworkError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub provider_id: String,
    pub display_name: String,
    pub plan_name: Option<String>,
    pub billing: BillingMode,
    pub balance: Option<Balance>,
    pub windows: Vec<QuotaWindow>,
    pub fidelity: Fidelity,
    pub status: HealthStatus,
    pub fetched_at: i64,
}

// ---------- 品牌 / 认证规格 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Brand {
    OpenAI,
    Kimi,
    Moonshot,
    DeepSeek,
    Tencent,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthField {
    pub key: &'static str,
    pub label: &'static str,
    pub placeholder: &'static str,
    pub secret: bool,
    pub required: bool,
    pub options: Option<Vec<(&'static str, &'static str)>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthSpec {
    // ⚠️ snake_case 会把 OAuth 拆成 "o_auth"，须显式重命名为 "oauth"（前端按此判断）
    #[serde(rename = "oauth")]
    OAuth {
        authorize_url: &'static str,
        token_url: &'static str,
        client_id: &'static str,
        scopes: &'static [&'static str],
        pkce: bool,
    },
    ApiKey {
        fields: Vec<AuthField>,
        hint: &'static str,
    },
    CloudSecret {
        fields: Vec<AuthField>,
    },
    Hybrid {
        primary: Box<AuthSpec>,
        fallback: Box<AuthSpec>,
    },
}

#[derive(Debug, Clone)]
pub struct Credential {
    /// 任意 JSON 序列化后的凭证内容（token / api key / secret pair）
    pub data: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct AuthInput {
    pub fields: HashMap<String, String>,
}

// ---------- Provider trait ----------

#[async_trait]
#[allow(async_fn_in_trait)]
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn brand(&self) -> Brand;
    fn billing_mode(&self) -> BillingMode;

    /// 驱动「添加供应商」表单的动态渲染
    fn auth_spec(&self) -> AuthSpec;

    /// 是否在「添加供应商」入口中可见。
    /// 默认 true。无官方 API 的平台（腾讯 Coding Plan、腾讯个人版 Token Plan）
    /// 暂返回 false 隐藏入口——代码保留，未来官方 API 提供时改回 true 即恢复。
    /// （2026-08-02 用户决策）
    fn enabled(&self) -> bool {
        true
    }

    /// 探测本机是否已有可复用凭证（Codex CLI / kimi CLI）
    async fn detect_local(&self) -> Option<Credential> {
        None
    }

    async fn authenticate(&self, _input: AuthInput) -> anyhow::Result<Credential> {
        anyhow::bail!("该 provider 暂不支持手动认证（请使用 detect_local 或 OAuth）")
    }

    async fn refresh(&self, _cred: &Credential) -> anyhow::Result<Option<Credential>> {
        Ok(None)
    }

    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot>;
}

/// 全部 8 个 provider 的注册表（M1 注册，M2/M3/M5 分批实现）
pub fn registry() -> Vec<Arc<dyn Provider>> {
    vec![
        Arc::new(moonshot::MoonshotProvider::new()),
        Arc::new(deepseek::DeepSeekProvider::new()),
        Arc::new(tencent_token_plan::TencentTokenPlanProvider::new()),
        Arc::new(tencent_tokenhub::TencentTokenHubProvider::new()),
        Arc::new(openai_platform::OpenAiPlatformProvider::new()),
        Arc::new(codex::CodexProvider::new()),
        Arc::new(kimi_code::KimiCodeProvider::new()),
        Arc::new(tencent_coding_plan::TencentCodingPlanProvider::new()),
    ]
}

/// 「添加供应商」入口可见的 provider（过滤掉无官方 API、暂隐藏的）。
/// 调度器仍用完整 `registry()`（已配置凭证的隐藏 provider 继续抓取）。
pub fn addable_registry() -> Vec<Arc<dyn Provider>> {
    registry().into_iter().filter(|p| p.enabled()).collect()
}

/// 跨平台用户主目录：macOS/Linux 用 $HOME，Windows 用 %USERPROFILE%。
/// detect_local 读 CLI 凭证文件（~/.codex、~/.kimi）时必须用它，否则 Windows 取空。
pub fn home_dir() -> std::path::PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}
