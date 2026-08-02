//! 腾讯 Coding Plan（订阅，请求次数计量，⚠️ 逆向，里程碑 M5）
//!
//! 官方未提供额度查询 API，三条候选路径（C1 逆向页面接口 / C2 响应头
//! x-ratelimit-* / C3 本地记账兜底）需实测确认。本阶段为编译占位，
//! 认证规格暂以 CloudSecret 占位（最终形态待 M5 定）。
//!
//! 合规提示：腾讯明令 Coding Plan「严禁 API 调用，仅限在编程工具中使用」。
//! 本 App 仅做只读额度查询、不产生推理调用；C2 依赖真实推理响应头，需重新评估。

use super::*;
use crate::core::providers::Brand;
use async_trait::async_trait;

pub struct TencentCodingPlanProvider;

impl TencentCodingPlanProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Provider for TencentCodingPlanProvider {
    fn id(&self) -> &'static str {
        "tencent_coding_plan"
    }
    fn display_name(&self) -> &'static str {
        "腾讯 Coding Plan"
    }
    fn brand(&self) -> Brand {
        Brand::Tencent
    }
    fn billing_mode(&self) -> BillingMode {
        BillingMode::Subscription
    }
    /// 无官方额度查询 API，隐藏「添加供应商」入口（2026-08-02 用户决策）。
    /// 代码保留，未来官方 API 提供时改回 true 即恢复。
    fn enabled(&self) -> bool {
        false
    }
    fn auth_spec(&self) -> AuthSpec {
        // 占位：最终形态待 M5（可能走 Hybrid / 本地记账）
        AuthSpec::CloudSecret {
            fields: vec![AuthField {
                key: "note",
                label: "状态",
                placeholder: "M5 待实现",
                secret: false,
                required: false,
                options: None,
            }],
        }
    }
    async fn fetch(&self, _cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        anyhow::bail!("腾讯 Coding Plan 额度查询未实现（里程碑 M5：C1/C2/C3 候选路径实测）")
    }
}
