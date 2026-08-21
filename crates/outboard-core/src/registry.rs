use crate::{
    CapturedOutput, IdentifierError, Manifest, ManifestError, Namespace, PluginId, PluginKind,
    PluginName, PluginRequirement, ProcessError,
    cache::{CacheMode, ManifestCache},
    discovery::{DiscoverySet, PluginCandidate, SearchRoot, build_roots, discover_in_roots},
    run_capture_args,
};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use thiserror::Error;
const TIMEOUT: Duration = Duration::from_secs(5);
#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("invalid Outboard identifier: {0}")]
    Identifier(#[from] IdentifierError),
    #[error("plugin discovery failed: {0}")]
    Discovery(#[source] std::io::Error),
    #[error("plugin process failed: {0}")]
    Process(#[from] ProcessError),
    #[error("plugin {path:?} exited unsuccessfully while reporting its manifest: {stderr}")]
    ManifestCommand { path: PathBuf, stderr: String },
    #[error("plugin {path:?} returned invalid manifest JSON: {source}")]
    ManifestJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("plugin {path:?} returned an invalid manifest: {source}")]
    ManifestInvalid {
        path: PathBuf,
        source: ManifestError,
    },
    #[error(
        "plugin path identity mismatch at {path:?}: discovered {discovered}, manifest declares {declared}"
    )]
    IdentityMismatch {
        path: PathBuf,
        discovered: PluginId,
        declared: PluginId,
    },
    #[error(
        "no plugin matched requirement for kind {kind}{name}; candidate rejections: {rejections:?}"
    )]
    NoMatch {
        kind: PluginKind,
        name: String,
        rejections: Vec<CandidateRejection>,
    },
}
#[derive(Debug, Clone)]
pub struct CandidateRejection {
    pub candidate: PluginCandidate,
    pub reason: String,
}
#[derive(Debug, Clone)]
pub struct InspectedPlugin {
    pub candidate: PluginCandidate,
    pub manifest: Manifest,
}
#[derive(Debug, Clone)]
pub struct ResolvedPlugin {
    pub candidate: PluginCandidate,
    pub manifest: Manifest,
}
impl ResolvedPlugin {
    pub fn id(&self) -> &PluginId {
        &self.manifest.plugin.id
    }
    pub fn path(&self) -> &Path {
        &self.candidate.path
    }
    pub fn command(&self) -> Command {
        Command::new(&self.candidate.path)
    }
    pub fn command_with(&self, args: impl IntoIterator<Item = OsString>) -> Command {
        let mut c = self.command();
        c.args(args);
        c
    }
    pub fn run_capture(
        &self,
        args: impl IntoIterator<Item = OsString>,
        input: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<CapturedOutput, ProcessError> {
        run_capture_args(&self.candidate.path, args, input, timeout)
    }
    pub fn from_parts(candidate: PluginCandidate, manifest: Manifest) -> Self {
        Self {
            candidate,
            manifest,
        }
    }
}
#[derive(Debug)]
pub struct Registry {
    namespace: Namespace,
    explicit_paths: Vec<PathBuf>,
    plugin_path_env: Option<String>,
    include_path: bool,
    overrides: BTreeMap<(PluginKind, PluginName), PathBuf>,
    cache: ManifestCache,
    control_timeout: Duration,
}
#[derive(Debug)]
pub struct RegistryBuilder {
    namespace: Namespace,
    explicit_paths: Vec<PathBuf>,
    plugin_path_env: Option<String>,
    include_path: bool,
    overrides: BTreeMap<(PluginKind, PluginName), PathBuf>,
    cache_mode: CacheMode,
    control_timeout: Duration,
}
impl Registry {
    pub fn builder(namespace: impl AsRef<str>) -> Result<RegistryBuilder, IdentifierError> {
        let n = Namespace::new(namespace.as_ref())?;
        let env = format!(
            "{}_PLUGIN_PATH",
            n.as_str().replace('-', '_').to_ascii_uppercase()
        );
        Ok(RegistryBuilder {
            namespace: n,
            explicit_paths: vec![],
            plugin_path_env: Some(env),
            include_path: true,
            overrides: BTreeMap::new(),
            cache_mode: CacheMode::default(),
            control_timeout: TIMEOUT,
        })
    }
    pub fn new(namespace: impl AsRef<str>) -> Result<Self, IdentifierError> {
        Ok(Self::builder(namespace)?.build())
    }
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }
    pub fn control_timeout(&self) -> Duration {
        self.control_timeout
    }
    pub fn cache(&self) -> &ManifestCache {
        &self.cache
    }
    pub fn search_roots(&self) -> Vec<SearchRoot> {
        build_roots(
            &self.explicit_paths,
            self.plugin_path_env.as_deref(),
            self.include_path,
        )
    }
    pub fn discover(&self, kind: impl AsRef<str>) -> Result<DiscoverySet, RegistryError> {
        let kind = PluginKind::new(kind.as_ref())?;
        let roots = self.search_roots();
        let overrides = self
            .overrides
            .iter()
            .filter(|((k, _), _)| k == &kind)
            .map(|((_, n), p)| (n.clone(), p.clone()));
        discover_in_roots(&self.namespace, &kind, roots, overrides)
            .map_err(RegistryError::Discovery)
    }
    pub fn inspect(&self, candidate: &PluginCandidate) -> Result<InspectedPlugin, RegistryError> {
        let manifest = if let Some(m) = self.cache.get(&candidate.path) {
            m
        } else {
            let m =
                inspect_manifest_at(candidate.id.clone(), &candidate.path, self.control_timeout)?;
            self.cache.insert(&candidate.path, m.clone());
            m
        };
        if manifest.plugin.id != candidate.id {
            self.cache.invalidate(&candidate.path);
            return Err(RegistryError::IdentityMismatch {
                path: candidate.path.clone(),
                discovered: candidate.id.clone(),
                declared: manifest.plugin.id,
            });
        }
        Ok(InspectedPlugin {
            candidate: candidate.clone(),
            manifest,
        })
    }
    pub fn resolve(&self, req: &PluginRequirement) -> Result<ResolvedPlugin, RegistryError> {
        self.resolve_best(req, |_| 0)
    }
    pub fn resolve_best<F>(
        &self,
        req: &PluginRequirement,
        score: F,
    ) -> Result<ResolvedPlugin, RegistryError>
    where
        F: Fn(&InspectedPlugin) -> i64,
    {
        let d = self.discover(req.kind.as_str())?;
        let mut rejected = Vec::new();
        let mut ok = Vec::new();
        for c in d
            .selected
            .into_iter()
            .filter(|c| req.name.as_ref().is_none_or(|n| &c.id.name == n))
        {
            match self.inspect(&c) {
                Err(e) => rejected.push(CandidateRejection {
                    candidate: c,
                    reason: e.to_string(),
                }),
                Ok(i) => {
                    let issues = i.manifest.compatibility_issues(req);
                    if issues.is_empty() {
                        ok.push((score(&i), i.candidate.precedence, i))
                    } else {
                        rejected.push(CandidateRejection {
                            candidate: i.candidate,
                            reason: issues
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join("; "),
                        })
                    }
                }
            }
        }
        ok.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        if let Some((_, _, i)) = ok.into_iter().next() {
            Ok(ResolvedPlugin {
                candidate: i.candidate,
                manifest: i.manifest,
            })
        } else {
            Err(RegistryError::NoMatch {
                kind: req.kind.clone(),
                name: req
                    .name
                    .as_ref()
                    .map_or_else(String::new, |n| format!(" named {n}")),
                rejections: rejected,
            })
        }
    }
}
impl RegistryBuilder {
    pub fn plugin_path(mut self, p: impl Into<PathBuf>) -> Self {
        self.explicit_paths.push(p.into());
        self
    }
    pub fn plugin_path_env(mut self, v: impl Into<String>) -> Self {
        self.plugin_path_env = Some(v.into());
        self
    }
    pub fn without_plugin_path_env(mut self) -> Self {
        self.plugin_path_env = None;
        self
    }
    pub fn include_path(mut self, v: bool) -> Self {
        self.include_path = v;
        self
    }
    pub fn override_plugin(
        mut self,
        kind: impl AsRef<str>,
        name: impl AsRef<str>,
        path: impl Into<PathBuf>,
    ) -> Result<Self, IdentifierError> {
        self.overrides.insert(
            (
                PluginKind::new(kind.as_ref())?,
                PluginName::new(name.as_ref())?,
            ),
            path.into(),
        );
        Ok(self)
    }
    pub fn cache(mut self, v: CacheMode) -> Self {
        self.cache_mode = v;
        self
    }
    pub fn control_timeout(mut self, v: Duration) -> Self {
        self.control_timeout = v;
        self
    }
    pub fn build(self) -> Registry {
        Registry {
            namespace: self.namespace,
            explicit_paths: self.explicit_paths,
            plugin_path_env: self.plugin_path_env,
            include_path: self.include_path,
            overrides: self.overrides,
            cache: ManifestCache::new(self.cache_mode),
            control_timeout: self.control_timeout,
        }
    }
}
pub fn inspect_manifest_at(
    id: PluginId,
    path: impl AsRef<OsStr>,
    timeout: Duration,
) -> Result<Manifest, RegistryError> {
    let pb = PathBuf::from(path.as_ref());
    let out = run_capture_args(path.as_ref(), ["__outboard", "manifest"], None, timeout)?;
    if !out.status.success() {
        return Err(RegistryError::ManifestCommand {
            path: pb,
            stderr: String::from_utf8_lossy(&out.stderr).trim().into(),
        });
    }
    let m: Manifest =
        serde_json::from_slice(&out.stdout).map_err(|source| RegistryError::ManifestJson {
            path: pb.clone(),
            source,
        })?;
    m.validate()
        .map_err(|source| RegistryError::ManifestInvalid {
            path: pb.clone(),
            source,
        })?;
    if m.plugin.id != id {
        return Err(RegistryError::IdentityMismatch {
            path: pb,
            discovered: id,
            declared: m.plugin.id,
        });
    }
    Ok(m)
}
