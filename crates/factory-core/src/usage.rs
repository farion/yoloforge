//! Observed OpenCode usage, not a billing estimate or a tokenizer approximation.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenLimits {
    pub work: Option<u64>,
    pub orchestration: Option<u64>,
    pub memory: Option<u64>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TokenBudget {
    pub project_id: crate::ProjectId,
    pub revision: crate::Revision,
    pub limits: TokenLimits,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache: CacheUsage,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CacheUsage {
    pub read: u64,
    pub write: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageGroup {
    pub role: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub assistant_messages: u64,
    pub unfinished_messages: u64,
    pub messages_missing_tokens: u64,
    pub messages_missing_cost: u64,
    pub tokens: TokenUsage,
    pub reported_cost_usd: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectUsage {
    #[serde(default)]
    pub token_budget: Option<TokenBudget>,
    pub project_id: crate::ProjectId,
    pub groups: Vec<UsageGroup>,
    /// No inference about missing historical messages or unreported provider usage.
    pub source: String,
}
