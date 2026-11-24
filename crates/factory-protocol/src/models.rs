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
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RoleModels {
    pub orchestrator: Option<ModelSelection>,
    pub implementer: Option<ModelSelection>,
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
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskModels {
    pub implementer_override: Option<ModelSelection>,
    pub orchestrator: Option<ModelSelection>,
    pub implementer: Option<ModelSelection>,
    #[serde(default)]
    pub supervisor: Option<ModelSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memorizer: Option<ModelSelection>,
}
