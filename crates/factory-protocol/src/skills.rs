use factory_core::{FactorySkillRef, RequestId};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

pub const FACTORY_SKILLS_CAPABILITY: &str = "factory-skills-v1";
pub const MAX_SKILL_BYTES: usize = 256 * 1024;
pub const MAX_SKILL_FILES: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FactorySkillSummary {
    pub reference: FactorySkillRef,
    pub name: String,
    pub description: String,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportFactorySkill {
    pub request_id: RequestId,
    pub id: String,
    pub name: String,
    pub description: String,
    /// Optimistic guard; a nonzero value means an explicit new version of this ID.
    pub current_version: u32,
    pub files: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
pub struct FactorySkillBundle {
    pub reference: FactorySkillRef,
    pub name: String,
    pub description: String,
    pub files: BTreeMap<String, String>,
}

pub fn valid_skill_files(files: &BTreeMap<String, String>) -> bool {
    if files.is_empty()
        || files.len() > MAX_SKILL_FILES
        || files.get("SKILL.md").is_none_or(|s| s.trim().is_empty())
    {
        return false;
    }
    let mut total = 0usize;
    for (name, body) in files {
        total = total.saturating_add(body.len());
        if total > MAX_SKILL_BYTES
            || name.len() > 200
            || name.contains('\\')
            || name.split('/').any(|part| {
                part.is_empty() || part.starts_with('.') || part.chars().any(char::is_control)
            })
            || Path::new(name)
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || (name != "SKILL.md" && name.ends_with("/SKILL.md"))
            || body.contains('\0')
        {
            return false;
        }
        if let Some((parent, _)) = name.rsplit_once('/') {
            if std::iter::successors(Some(parent), |prefix| {
                prefix.rsplit_once('/').map(|(parent, _)| parent)
            })
            .any(|prefix| files.contains_key(prefix))
            {
                return false;
            }
        }
    }
    true
}
