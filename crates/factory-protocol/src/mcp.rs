use factory_core::{LocalMcp, RemoteMcpRef, RequestId, Revision};
use serde::{Deserialize, Serialize};

pub const LOCAL_MCP_CAPABILITY: &str = "local-mcp-v1";
pub const REMOTE_MCP_CAPABILITY: &str = "remote-mcp-v1";
pub const LOCAL_MCP_PROBE_CAPABILITY: &str = "local-mcp-probe-v1";
pub const MCP_OAUTH_LOOPBACK_PORT: u16 = 52818;
pub const MCP_OAUTH_LOOPBACK_REDIRECT: &str = "http://127.0.0.1:52818/mcp-oauth/callback";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalMcpProbeStatus {
    Pending,
    Connected,
    ProgramMissing,
    Unavailable,
}

#[derive(Serialize, Deserialize)]
pub struct LocalMcpProbe {
    pub id: RequestId,
    pub command: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct LocalMcpProbeView {
    pub id: RequestId,
    pub status: LocalMcpProbeStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RemoteMcpAuth {
    None,
    Header {
        name: String,
        bearer: bool,
    },
    OAuth {
        client_id: String,
        authorization_url: String,
        token_url: String,
        redirect_uri: String,
        #[serde(default)]
        scope: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteMcp {
    #[serde(flatten)]
    pub reference: RemoteMcpRef,
    pub url: String,
    pub auth: RemoteMcpAuth,
    pub has_key: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveRemoteMcp {
    pub expected_revision: Revision,
    pub name: String,
    pub url: String,
    pub auth: RemoteMcpAuth,
    /// Missing when keeping the existing secret. Never serialize this into a request journal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remove_key: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveRemoteMcp {
    pub request_id: RequestId,
    pub expected_revision: Revision,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeRemoteMcpAuth {
    pub url: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RemoteMcpAuthDiscovery {
    Open,
    OAuth { auth: RemoteMcpAuth },
    Unknown,
    Unavailable,
}

#[derive(Serialize, Deserialize)]
pub struct RemoteMcpLease {
    pub path: String,
    pub token: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalMcpCatalog {
    pub revision: Revision,
    pub entries: Vec<LocalMcp>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetLocalMcpCatalog {
    pub request_id: RequestId,
    pub catalog: LocalMcpCatalog,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectLocalMcps {
    pub revision: Revision,
    pub required: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetProjectLocalMcps {
    pub request_id: RequestId,
    pub settings: ProjectLocalMcps,
}
