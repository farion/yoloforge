//! Shared SSH command construction, with the terminal-specific settings editor.
use anyhow::{ensure, Context, Result};
pub use factory_client_core::ssh::{
    command, config_path, configured, fetch_repository, project_readiness, publish_review, Profile,
};
use factory_core::{CommandId, RunnerId};
use std::{collections::BTreeMap, path::PathBuf};
mod editor;
pub use editor::Dialog;
