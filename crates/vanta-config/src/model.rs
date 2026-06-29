//! The typed model for `vanta.toml` and `~/.vanta/config.toml`.
//!
//! See `docs/05-configuration.md` and `docs/27-config-reference.md`. Unknown keys
//! are rejected (`deny_unknown_fields`) so typos become span-accurate diagnostics.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A project manifest (`vanta.toml`) or global config (`~/.vanta/config.toml`).
/// Both share the same shape; only some tables are meaningful in each context.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Manifest {
    /// Manifest format version marker.
    pub version: Option<u32>,
    /// The declared tools.
    pub tools: BTreeMap<String, ToolSpec>,
    /// Environment variables injected when active (a trusted section).
    pub env: BTreeMap<String, EnvValue>,
    /// The optional task runner.
    pub tasks: BTreeMap<String, Task>,
    /// Behavioral settings.
    pub settings: Settings,
    /// Named registries that overlay the official one.
    pub registries: BTreeMap<String, Registry>,
    /// Workspace declaration (root manifest only).
    pub workspace: Option<Workspace>,
}

/// A tool entry: either a bare version string or a detailed table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolSpec {
    Version(String),
    Detailed(ToolDetail),
}

impl ToolSpec {
    /// The version request string for this tool.
    pub fn version(&self) -> &str {
        match self {
            ToolSpec::Version(v) => v,
            ToolSpec::Detailed(d) => &d.version,
        }
    }
}

/// The inline-table form of a tool entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDetail {
    pub version: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub registry: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub os: Option<Vec<String>>,
    #[serde(default)]
    pub optional: Option<bool>,
}

/// An `[env]` value: a plain string or a PATH edit table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EnvValue {
    Plain(String),
    Path(PathEdit),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathEdit {
    #[serde(default)]
    pub prepend: Vec<String>,
    #[serde(default)]
    pub append: Vec<String>,
}

/// A `[tasks]` entry: a command string or a detailed table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Task {
    Command(String),
    Detailed(TaskDetail),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDetail {
    pub run: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

/// Behavioral settings. All optional so merge is "higher layer wins if set".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub auto_install: Option<bool>,
    pub verify: Option<String>,
    pub jobs: Option<u32>,
    pub link_strategy: Option<String>,
    pub shims: Option<bool>,
    pub offline: Option<bool>,
    pub mirror: Option<String>,
    pub targets: Option<Vec<String>>,
    pub retain_generations: Option<u32>,
    pub gc_keep_days: Option<u32>,
    pub color: Option<String>,
    pub telemetry: Option<bool>,
    pub read_foreign_versions: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub url: String,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub auth: Option<String>,
    #[serde(default)]
    pub public_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub members: Vec<String>,
    #[serde(default)]
    pub inherit: Option<bool>,
}
