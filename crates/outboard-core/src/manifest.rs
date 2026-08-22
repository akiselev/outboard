use crate::{CapabilityId, IdentifierError, InterfaceId, PluginId, PluginKind, PluginName};
use schemars::JsonSchema;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

pub const MANIFEST_VERSION: u32 = 1;
pub const OUTBOARD_FRAMEWORK_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const OUTBOARD_PROTOCOL_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PluginMetadata {
    pub id: PluginId,
    pub version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
}
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    OneShot,
    Worker,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InterfaceManifest {
    pub id: InterfaceId,
    pub version: Version,
}
impl InterfaceManifest {
    pub fn new(id: impl AsRef<str>, version: Version) -> Result<Self, IdentifierError> {
        Ok(Self {
            id: InterfaceId::new(id.as_ref())?,
            version,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Capability {
    pub id: CapabilityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<Version>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, Value>,
}
impl Capability {
    pub fn new(id: impl AsRef<str>) -> Result<Self, IdentifierError> {
        Ok(Self {
            id: CapabilityId::new(id.as_ref())?,
            version: None,
            properties: BTreeMap::new(),
        })
    }
    pub fn version(mut self, version: Version) -> Self {
        self.version = Some(version);
        self
    }
    pub fn property(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.properties.insert(key.into(), value.into());
        self
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ControlCapabilities {
    #[serde(default = "yes")]
    pub doctor: bool,
    #[serde(default)]
    pub cli_schema: bool,
    #[serde(default = "yes")]
    pub ping: bool,
}
const fn yes() -> bool {
    true
}
impl Default for ControlCapabilities {
    fn default() -> Self {
        Self {
            doctor: true,
            cli_schema: false,
            ping: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Manifest {
    pub manifest_version: u32,
    pub plugin: PluginMetadata,
    #[schemars(with = "String")]
    pub framework: VersionReq,
    #[schemars(with = "String")]
    pub worker_protocol: VersionReq,
    #[serde(default)]
    pub interfaces: Vec<InterfaceManifest>,
    #[serde(default)]
    pub execution: Vec<ExecutionMode>,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub control: ControlCapabilities,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ManifestError {
    #[error("unsupported manifest version {0}; host supports {MANIFEST_VERSION}")]
    UnsupportedManifestVersion(u32),
    #[error("plugin manifest has no execution modes")]
    MissingExecutionMode,
    #[error("plugin manifest contains duplicate interface {0}")]
    DuplicateInterface(String),
    #[error("plugin manifest contains duplicate capability {0}")]
    DuplicateCapability(String),
    #[error("invalid identifier in manifest: {0}")]
    InvalidIdentifier(#[from] IdentifierError),
}
impl Manifest {
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.manifest_version != MANIFEST_VERSION {
            return Err(ManifestError::UnsupportedManifestVersion(
                self.manifest_version,
            ));
        }
        PluginId::new(
            self.plugin.id.namespace.as_str(),
            self.plugin.id.kind.as_str(),
            self.plugin.id.name.as_str(),
        )?;
        if self.execution.is_empty() {
            return Err(ManifestError::MissingExecutionMode);
        }
        let mut ids = std::collections::BTreeSet::new();
        for i in &self.interfaces {
            InterfaceId::new(i.id.as_str())?;
            if !ids.insert(i.id.as_str()) {
                return Err(ManifestError::DuplicateInterface(i.id.to_string()));
            }
        }
        let mut caps = std::collections::BTreeSet::new();
        for c in &self.capabilities {
            CapabilityId::new(c.id.as_str())?;
            if !caps.insert(c.id.as_str()) {
                return Err(ManifestError::DuplicateCapability(c.id.to_string()));
            }
        }
        Ok(())
    }
    pub fn interface(&self, id: &InterfaceId) -> Option<&InterfaceManifest> {
        self.interfaces.iter().find(|x| &x.id == id)
    }
    pub fn capability(&self, id: &CapabilityId) -> Option<&Capability> {
        self.capabilities.iter().find(|x| &x.id == id)
    }
    pub fn supports(&self, mode: ExecutionMode) -> bool {
        self.execution.contains(&mode)
    }
    pub fn compatibility_issues(&self, req: &PluginRequirement) -> Vec<CompatibilityIssue> {
        let mut out = Vec::new();
        let host = Version::parse(OUTBOARD_FRAMEWORK_VERSION).expect("package version is semver");
        if !self.framework.matches(&host) {
            out.push(CompatibilityIssue::Framework {
                plugin_accepts: self.framework.clone(),
                host,
            });
        }
        if let Some(name) = &req.name {
            if &self.plugin.id.name != name {
                out.push(CompatibilityIssue::WrongName {
                    expected: name.clone(),
                    actual: self.plugin.id.name.clone(),
                });
            }
        }
        if self.plugin.id.kind != req.kind {
            out.push(CompatibilityIssue::WrongKind {
                expected: req.kind.clone(),
                actual: self.plugin.id.kind.clone(),
            });
        }
        if let Some(mode) = req.execution {
            if !self.supports(mode) {
                out.push(CompatibilityIssue::ExecutionMode(mode));
            }
        }
        if let Some(r) = &req.interface {
            match self.interface(&r.id) {
                None => out.push(CompatibilityIssue::MissingInterface(r.id.clone())),
                Some(i) if !r.version.matches(&i.version) => {
                    out.push(CompatibilityIssue::InterfaceVersion {
                        id: r.id.clone(),
                        required: r.version.clone(),
                        actual: i.version.clone(),
                    })
                }
                Some(_) => {}
            }
        }
        for r in &req.capabilities {
            match self.capability(&r.id) {
                None => out.push(CompatibilityIssue::MissingCapability(r.id.clone())),
                Some(c) => {
                    if let Some(vr) = &r.version {
                        if c.version.as_ref().is_none_or(|v| !vr.matches(v)) {
                            out.push(CompatibilityIssue::CapabilityVersion {
                                id: r.id.clone(),
                                required: vr.clone(),
                                actual: c.version.clone(),
                            });
                        }
                    }
                    for (k, expected) in &r.properties {
                        if c.properties.get(k) != Some(expected) {
                            out.push(CompatibilityIssue::CapabilityProperty {
                                id: r.id.clone(),
                                key: k.clone(),
                                expected: expected.clone(),
                                actual: c.properties.get(k).cloned(),
                            });
                        }
                    }
                }
            }
        }
        if req.execution == Some(ExecutionMode::Worker) {
            let protocol = Version::parse(OUTBOARD_PROTOCOL_VERSION).expect("protocol is semver");
            if !self.worker_protocol.matches(&protocol) {
                out.push(CompatibilityIssue::WorkerProtocol {
                    plugin_accepts: self.worker_protocol.clone(),
                    host: protocol,
                });
            }
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct ManifestBuilder {
    manifest: Manifest,
}
impl ManifestBuilder {
    pub fn new(id: PluginId, version: Version) -> Self {
        Self {
            manifest: Manifest {
                manifest_version: MANIFEST_VERSION,
                plugin: PluginMetadata {
                    id,
                    version,
                    description: None,
                    homepage: None,
                },
                framework: VersionReq::parse(&format!("^{}", OUTBOARD_FRAMEWORK_VERSION)).unwrap(),
                worker_protocol: VersionReq::parse("^1.0").unwrap(),
                interfaces: vec![],
                execution: vec![ExecutionMode::OneShot],
                capabilities: vec![],
                control: ControlCapabilities::default(),
                metadata: BTreeMap::new(),
            },
        }
    }
    pub fn description(mut self, v: impl Into<String>) -> Self {
        self.manifest.plugin.description = Some(v.into());
        self
    }
    pub fn homepage(mut self, v: impl Into<String>) -> Self {
        self.manifest.plugin.homepage = Some(v.into());
        self
    }
    pub fn framework(mut self, v: VersionReq) -> Self {
        self.manifest.framework = v;
        self
    }
    pub fn worker_protocol(mut self, v: VersionReq) -> Self {
        self.manifest.worker_protocol = v;
        self
    }
    pub fn interface(mut self, v: InterfaceManifest) -> Self {
        self.manifest.interfaces.push(v);
        self
    }
    pub fn capability(mut self, v: Capability) -> Self {
        self.manifest.capabilities.push(v);
        self
    }
    pub fn execution(mut self, v: impl IntoIterator<Item = ExecutionMode>) -> Self {
        self.manifest.execution = v.into_iter().collect();
        self
    }
    pub fn control(mut self, v: ControlCapabilities) -> Self {
        self.manifest.control = v;
        self
    }
    pub fn metadata(mut self, k: impl Into<String>, v: impl Into<Value>) -> Self {
        self.manifest.metadata.insert(k.into(), v.into());
        self
    }
    pub fn build(self) -> Result<Manifest, ManifestError> {
        self.manifest.validate()?;
        Ok(self.manifest)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InterfaceRequirement {
    pub id: InterfaceId,
    #[schemars(with = "String")]
    pub version: VersionReq,
}
impl InterfaceRequirement {
    pub fn new(id: impl AsRef<str>, version: VersionReq) -> Result<Self, IdentifierError> {
        Ok(Self {
            id: InterfaceId::new(id.as_ref())?,
            version,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CapabilityRequirement {
    pub id: CapabilityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub version: Option<VersionReq>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, Value>,
}
impl CapabilityRequirement {
    pub fn new(id: impl AsRef<str>) -> Result<Self, IdentifierError> {
        Ok(Self {
            id: CapabilityId::new(id.as_ref())?,
            version: None,
            properties: BTreeMap::new(),
        })
    }
    pub fn version(mut self, v: VersionReq) -> Self {
        self.version = Some(v);
        self
    }
    pub fn property(mut self, k: impl Into<String>, v: impl Into<Value>) -> Self {
        self.properties.insert(k.into(), v.into());
        self
    }
}
#[derive(Debug, Clone)]
pub struct PluginRequirement {
    pub kind: PluginKind,
    pub name: Option<PluginName>,
    pub interface: Option<InterfaceRequirement>,
    pub capabilities: Vec<CapabilityRequirement>,
    pub execution: Option<ExecutionMode>,
}
impl PluginRequirement {
    pub fn new(kind: impl AsRef<str>) -> Result<Self, IdentifierError> {
        Ok(Self {
            kind: PluginKind::new(kind.as_ref())?,
            name: None,
            interface: None,
            capabilities: vec![],
            execution: None,
        })
    }
    pub fn named(mut self, name: impl AsRef<str>) -> Result<Self, IdentifierError> {
        self.name = Some(PluginName::new(name.as_ref())?);
        Ok(self)
    }
    pub fn interface(mut self, v: InterfaceRequirement) -> Self {
        self.interface = Some(v);
        self
    }
    pub fn capability(mut self, v: CapabilityRequirement) -> Self {
        self.capabilities.push(v);
        self
    }
    pub fn execution(mut self, v: ExecutionMode) -> Self {
        self.execution = Some(v);
        self
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum CompatibilityIssue {
    Framework {
        plugin_accepts: VersionReq,
        host: Version,
    },
    WorkerProtocol {
        plugin_accepts: VersionReq,
        host: Version,
    },
    WrongName {
        expected: PluginName,
        actual: PluginName,
    },
    WrongKind {
        expected: PluginKind,
        actual: PluginKind,
    },
    ExecutionMode(ExecutionMode),
    MissingInterface(InterfaceId),
    InterfaceVersion {
        id: InterfaceId,
        required: VersionReq,
        actual: Version,
    },
    MissingCapability(CapabilityId),
    CapabilityVersion {
        id: CapabilityId,
        required: VersionReq,
        actual: Option<Version>,
    },
    CapabilityProperty {
        id: CapabilityId,
        key: String,
        expected: Value,
        actual: Option<Value>,
    },
}
impl std::fmt::Display for CompatibilityIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Framework {
                plugin_accepts,
                host,
            } => write!(
                f,
                "plugin accepts Outboard {plugin_accepts}, host is {host}"
            ),
            Self::WorkerProtocol {
                plugin_accepts,
                host,
            } => write!(
                f,
                "plugin accepts worker protocol {plugin_accepts}, host uses {host}"
            ),
            Self::WrongName { expected, actual } => {
                write!(f, "expected plugin {expected}, found {actual}")
            }
            Self::WrongKind { expected, actual } => {
                write!(f, "expected kind {expected}, found {actual}")
            }
            Self::ExecutionMode(m) => write!(f, "plugin does not support execution mode {m:?}"),
            Self::MissingInterface(id) => write!(f, "plugin does not provide interface {id}"),
            Self::InterfaceVersion {
                id,
                required,
                actual,
            } => write!(
                f,
                "interface {id} requires {required}, plugin provides {actual}"
            ),
            Self::MissingCapability(id) => write!(f, "plugin is missing capability {id}"),
            Self::CapabilityVersion {
                id,
                required,
                actual,
            } => write!(
                f,
                "capability {id} requires {required}, plugin provides {actual:?}"
            ),
            Self::CapabilityProperty {
                id,
                key,
                expected,
                actual,
            } => write!(
                f,
                "capability {id} property {key} requires {expected}, plugin provides {actual:?}"
            ),
        }
    }
}
