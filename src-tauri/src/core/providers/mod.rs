//! Provider 抽象层
//!
//! 把各平台的差异收敛到统一的 `Provider` trait 与数据模型，
//! UI 层与调度器不感知具体平台。新增平台只需实现 trait，
//! 「添加供应商」表单由 `auth_spec()` 数据驱动自动渲染。
#![allow(dead_code)] // Provider API surface; consumed by M2/M3 add-provider UI, not yet read in M1

pub mod anthropic_api;
pub mod claude;
pub mod codex;
pub mod deepseek;
pub mod gemini;
pub mod glm_api;
pub mod glm_coding_plan;
pub mod kimi_code;
pub mod minimax_api;
pub mod minimax_token_plan;
pub mod moonshot;
pub mod openai_platform;
pub mod openrouter;
pub mod presentation;
pub mod siliconflow;
pub mod tencent;
pub mod tencent_coding_plan;
pub mod tencent_token_plan;
pub mod tencent_tokenhub;
pub mod volcengine;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

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
    /// 用量百分比（0-100）。无上限/成本型窗口为 None。
    pub used: Option<f64>,
    /// 原始用量（金额 / token 数 / 请求数），用于无上限窗口展示与有上限窗口的数值文案。
    #[serde(default)]
    pub used_raw: Option<f64>,
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
    /// 账号实例 ID。provider 实现只负责平台数据，调度器会在抓取后填入实际账号 ID。
    #[serde(default)]
    pub account_id: String,
    /// 适合展示的账号标识（邮箱/姓名/平台用户 ID 等）。没有时前端不显示占位名。
    #[serde(default)]
    pub account_label: Option<String>,
    pub provider_id: String,
    pub display_name: String,
    pub plan_name: Option<String>,
    pub billing: BillingMode,
    pub balance: Option<Balance>,
    pub windows: Vec<QuotaWindow>,
    pub fidelity: Fidelity,
    pub status: HealthStatus,
    pub fetched_at: i64,
    /// 最近一次抓取失败的说明（成功时为 None）。失败时快照保留旧数据，仅更新此字段与 status。
    #[serde(default)]
    pub last_error: Option<String>,
}

// ---------- 品牌 / 认证规格 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Brand {
    Anthropic,
    OpenAI,
    OpenRouter,
    Kimi,
    Moonshot,
    DeepSeek,
    Glm,
    MiniMax,
    Tencent,
    Gemini,
    SiliconFlow,
    Volcengine,
}

/// “添加供应商”界面的一级厂商分组。
/// Brand 用于具体产品卡片配色；Vendor 用于把同一厂商的 Plan / API 入口归在一起。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Anthropic,
    OpenAI,
    OpenRouter,
    Moonshot,
    DeepSeek,
    Glm,
    MiniMax,
    Tencent,
    Google,
    SiliconFlow,
    Volcengine,
}

impl Brand {
    pub fn vendor(self) -> Vendor {
        match self {
            Brand::Anthropic => Vendor::Anthropic,
            Brand::OpenAI => Vendor::OpenAI,
            Brand::OpenRouter => Vendor::OpenRouter,
            Brand::Kimi | Brand::Moonshot => Vendor::Moonshot,
            Brand::DeepSeek => Vendor::DeepSeek,
            Brand::Glm => Vendor::Glm,
            Brand::MiniMax => Vendor::MiniMax,
            Brand::Tencent => Vendor::Tencent,
            Brand::Gemini => Vendor::Google,
            Brand::SiliconFlow => Vendor::SiliconFlow,
            Brand::Volcengine => Vendor::Volcengine,
        }
    }
}

impl Vendor {
    pub fn id(self) -> &'static str {
        match self {
            Vendor::Anthropic => "anthropic",
            Vendor::OpenAI => "openai",
            Vendor::OpenRouter => "openrouter",
            Vendor::Moonshot => "moonshot",
            Vendor::DeepSeek => "deepseek",
            Vendor::Glm => "glm",
            Vendor::MiniMax => "minimax",
            Vendor::Tencent => "tencent",
            Vendor::Google => "google",
            Vendor::SiliconFlow => "siliconflow",
            Vendor::Volcengine => "volcengine",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Vendor::Anthropic => "Anthropic",
            Vendor::OpenAI => "OpenAI",
            Vendor::OpenRouter => "OpenRouter",
            Vendor::Moonshot => "Moonshot",
            Vendor::DeepSeek => "DeepSeek",
            Vendor::Glm => "GLM",
            Vendor::MiniMax => "MiniMax",
            Vendor::Tencent => "腾讯云",
            Vendor::Google => "Google",
            Vendor::SiliconFlow => "SiliconFlow",
            Vendor::Volcengine => "火山引擎",
        }
    }

    /// 厂商栏目使用统一图标；Moonshot 采用用户更熟悉的 Kimi 标识。
    pub fn brand(self) -> Brand {
        match self {
            Vendor::Anthropic => Brand::Anthropic,
            Vendor::OpenAI => Brand::OpenAI,
            Vendor::OpenRouter => Brand::OpenRouter,
            Vendor::Moonshot => Brand::Kimi,
            Vendor::DeepSeek => Brand::DeepSeek,
            Vendor::Glm => Brand::Glm,
            Vendor::MiniMax => Brand::MiniMax,
            Vendor::Tencent => Brand::Tencent,
            Vendor::Google => Brand::Gemini,
            Vendor::SiliconFlow => Brand::SiliconFlow,
            Vendor::Volcengine => Brand::Volcengine,
        }
    }

    pub fn order(self) -> u8 {
        match self {
            Vendor::OpenAI => 0,
            Vendor::Anthropic => 1,
            Vendor::OpenRouter => 2,
            Vendor::Moonshot => 3,
            Vendor::DeepSeek => 4,
            Vendor::Glm => 5,
            Vendor::MiniMax => 6,
            Vendor::Tencent => 7,
            Vendor::Google => 8,
            Vendor::Volcengine => 9,
            Vendor::SiliconFlow => 10,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddAccountType {
    Plan,
    Api,
}

impl AddAccountType {
    pub fn from_billing(mode: BillingMode) -> Self {
        match mode {
            BillingMode::Subscription => Self::Plan,
            BillingMode::PayAsYouGo => Self::Api,
        }
    }

    pub fn order(self) -> u8 {
        match self {
            Self::Plan => 0,
            Self::Api => 1,
        }
    }
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

    /// 添加界面中的产品名称。与主卡片标题分开，允许使用“Codex 套餐”等动作语义。
    fn add_product_name(&self) -> &'static str {
        self.display_name()
    }

    /// 添加界面中的一行简短说明。
    fn add_description(&self) -> &'static str {
        match self.billing_mode() {
            BillingMode::Subscription => "套餐额度",
            BillingMode::PayAsYouGo => "按量余额与用量",
        }
    }

    /// 添加入口中的账户产品类型。它与认证方式完全独立：Plan 也可以使用 API Key。
    fn add_account_type(&self) -> AddAccountType {
        AddAccountType::from_billing(self.billing_mode())
    }

    /// 控制台详情链接；作为标准卡片契约的一部分传给前端。
    fn detail_url(&self) -> Option<&'static str> {
        None
    }

    /// 供应商字段到标准卡片语义的映射配置。
    fn card_config(&self) -> presentation::CardConfig {
        let mut config = presentation::CardConfig::for_billing(self.billing_mode());
        config.detail_url = self.detail_url();
        config
    }

    /// 套餐按大致月价映射到统一 0–5 档；未知/合同价返回 None。
    fn plan_tier(&self, _plan_name: &str) -> Option<u8> {
        None
    }

    /// ProviderSnapshot → 前端唯一消费的标准卡片模型。
    fn present(&self, snapshot: &ProviderSnapshot) -> presentation::AccountCardModel {
        let tier = snapshot
            .plan_name
            .as_deref()
            .and_then(|name| self.plan_tier(name));
        presentation::present(snapshot, self.brand(), self.card_config(), tier)
    }

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

    /// 添加表单是否应展示“一键导入本机凭证”。与认证类型分离：
    /// API Key 型产品也可能有官方 CLI / 桌面端凭证可复用。
    fn supports_local_import(&self) -> bool {
        false
    }

    async fn authenticate(&self, _input: AuthInput) -> anyhow::Result<Credential> {
        anyhow::bail!("该 provider 暂不支持手动认证（请使用 detect_local 或 OAuth）")
    }

    async fn refresh(&self, _cred: &Credential) -> anyhow::Result<Option<Credential>> {
        Ok(None)
    }

    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot>;
}

/// 全部 provider 的注册表。添加界面与调度器都从这里取得实现。
pub fn registry() -> Vec<Arc<dyn Provider>> {
    vec![
        Arc::new(claude::ClaudeProvider::new()),
        Arc::new(anthropic_api::AnthropicApiProvider::new()),
        Arc::new(moonshot::MoonshotProvider::new()),
        Arc::new(deepseek::DeepSeekProvider::new()),
        Arc::new(glm_coding_plan::GlmCodingPlanProvider::new()),
        Arc::new(glm_api::GlmApiProvider::new()),
        Arc::new(minimax_token_plan::MiniMaxTokenPlanProvider::new()),
        Arc::new(minimax_api::MiniMaxApiProvider::new()),
        Arc::new(tencent_token_plan::TencentTokenPlanProvider::new()),
        Arc::new(tencent_tokenhub::TencentTokenHubProvider::new()),
        Arc::new(openai_platform::OpenAiPlatformProvider::new()),
        Arc::new(openrouter::OpenRouterProvider::new()),
        Arc::new(codex::CodexProvider::new()),
        Arc::new(kimi_code::KimiCodeProvider::new()),
        Arc::new(tencent_coding_plan::TencentCodingPlanProvider::new()),
        Arc::new(gemini::GeminiProvider::new()),
        Arc::new(volcengine::VolcengineProvider::new()),
        Arc::new(siliconflow::SiliconFlowProvider::new()),
    ]
}

/// 「添加供应商」入口可见的 provider（过滤掉无官方 API、暂隐藏的）。
/// 调度器仍用完整 `registry()`（已配置凭证的隐藏 provider 继续抓取）。
pub fn addable_registry() -> Vec<Arc<dyn Provider>> {
    registry().into_iter().filter(|p| p.enabled()).collect()
}

/// 统一的 HTTP 客户端：所有 provider / OAuth 流程共用，带连接与总超时。
/// 之前每个请求都 `Client::new()`，默认无超时，一个不响应的连接会永久挂死调度器。
pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()
            .expect("构建 HTTP 客户端失败")
    })
}

/// 跨平台用户主目录：macOS/Linux 用 $HOME，Windows 用 %USERPROFILE%。
/// detect_local 读 CLI 凭证文件（~/.codex、~/.kimi）时必须用它，否则 Windows 取空。
pub fn home_dir() -> std::path::PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}
