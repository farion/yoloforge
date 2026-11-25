use crate::{RequestId, Revision, SubmitTask};
use serde::{Deserialize, Serialize};

/// An access ID is a stable Factory provider alias, never a credential.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelSelection {
    pub access_id: String,
    pub model_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelAccess {
    pub id: String,
    pub name: String,
    pub models: Vec<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub transports: std::collections::BTreeMap<String, ModelTransport>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelTransport {
    Chat,
    Responses,
    Messages,
}

impl ModelTransport {
    pub fn runtime_package(self) -> &'static str {
        match self {
            Self::Chat => "@opencode/ai/providers/openai-compatible",
            Self::Responses => "@opencode/ai/providers/openai-compatible/responses",
            Self::Messages => "@opencode/ai/providers/anthropic",
        }
    }
}

pub const COPILOT_INTEGRATION: &str = "github-copilot";
pub const COPILOT_ACCESS_PREFIX: &str = "factory-github-copilot-";

pub fn is_copilot_access(id: &str) -> bool {
    id.strip_prefix(COPILOT_ACCESS_PREFIX).is_some_and(|suffix| {
        suffix.parse::<RequestId>().is_ok_and(|id| id.to_string() == suffix)
    })
}

/// API-key providers use reserved alias prefixes so older access records and
/// ChatGPT selections retain their existing wire format and model IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyModelProvider {
    Gemini,
    OpenRouter,
    Anthropic,
    LiteLlm,
    Ollama,
}

impl KeyModelProvider {
    pub const ALL: [Self; 5] = [
        Self::Gemini,
        Self::OpenRouter,
        Self::Anthropic,
        Self::LiteLlm,
        Self::Ollama,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Gemini => "Google Gemini",
            Self::OpenRouter => "OpenRouter",
            Self::Anthropic => "Anthropic (Claude)",
            Self::LiteLlm => "LiteLLM",
            Self::Ollama => "Ollama",
        }
    }

    pub fn integration(self) -> &'static str {
        match self {
            Self::Gemini => "google",
            Self::OpenRouter => "openrouter",
            Self::Anthropic => "anthropic",
            Self::LiteLlm => "litellm",
            Self::Ollama => "ollama",
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Gemini => "factory-google-",
            Self::OpenRouter => "factory-openrouter-",
            Self::Anthropic => "factory-anthropic-",
            Self::LiteLlm => "factory-litellm-",
            Self::Ollama => "factory-ollama-",
        }
    }

    pub fn new_access_id(self) -> String {
        format!("{}{}", self.prefix(), RequestId::new())
    }

    pub fn from_access_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|provider| {
            id.strip_prefix(provider.prefix()).is_some_and(|suffix| {
                suffix
                    .parse::<RequestId>()
                    .is_ok_and(|id| id.to_string() == suffix)
            })
        })
    }

    pub fn runtime_package(self) -> &'static str {
        match self {
            Self::Gemini => "@opencode/ai/providers/google",
            Self::OpenRouter => "@opencode/ai/providers/openrouter",
            Self::Anthropic => "@opencode/ai/providers/anthropic",
            Self::LiteLlm | Self::Ollama => "@opencode/ai/providers/openai-compatible",
        }
    }

    pub fn endpoint_env(self) -> Option<&'static str> {
        match self {
            Self::LiteLlm => Some("FACTORY_LITELLM_URL"),
            Self::Ollama => Some("FACTORY_OLLAMA_URL"),
            _ => None,
        }
    }

    pub fn key_optional(self) -> bool {
        self == Self::Ollama
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RoleModels {
    pub orchestrator: Option<ModelSelection>,
    pub implementer: Option<ModelSelection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<ModelSelection>,
    pub supervisor: Option<ModelSelection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memorizer: Option<ModelSelection>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FactoryModelSettings {
    pub revision: Revision,
    pub default_model: Option<ModelSelection>,
    #[serde(default)]
    pub roles: RoleModels,
    #[serde(default)]
    pub accesses: Vec<ModelAccess>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetFactoryModels {
    pub request_id: RequestId,
    pub settings: FactoryModelSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectModelSettings {
    pub revision: Revision,
    pub roles: RoleModels,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetProjectModels {
    pub request_id: RequestId,
    pub settings: ProjectModelSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitTaskWithModel {
    pub submission: SubmitTask,
    /// Null means Default (inherit); save the effective pair only on release.
    pub implementer: Option<ModelSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_mcps: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskModels {
    pub implementer_override: Option<ModelSelection>,
    pub orchestrator: Option<ModelSelection>,
    pub implementer: Option<ModelSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<ModelSelection>,
    #[serde(default)]
    pub supervisor: Option<ModelSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memorizer: Option<ModelSelection>,
}
