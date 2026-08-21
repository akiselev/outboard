//! Black-box conformance checks that execute a plugin exactly as a host would.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use outboard::{
    DoctorCheck, DoctorReport, DoctorStatus, ExecutionMode, InterfaceRequirement, Manifest,
    PluginCandidate, PluginId, ResolvedPlugin, WorkerClient, run_capture_args,
};
use semver::VersionReq;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ConformanceOptions {
    pub expected_id: Option<PluginId>,
    pub required_interfaces: Vec<InterfaceRequirement>,
    pub require_worker: bool,
    pub require_doctor: bool,
    pub require_cli_schema: bool,
    pub timeout: Duration,
}

impl Default for ConformanceOptions {
    fn default() -> Self {
        Self {
            expected_id: None,
            required_interfaces: vec![],
            require_worker: false,
            require_doctor: false,
            require_cli_schema: false,
            timeout: Duration::from_secs(10),
        }
    }
}

impl ConformanceOptions {
    pub fn expected_id(mut self, id: PluginId) -> Self {
        self.expected_id = Some(id);
        self
    }

    pub fn require_interface(
        mut self,
        id: impl AsRef<str>,
        version: &str,
    ) -> Result<Self, ConformanceError> {
        self.required_interfaces
            .push(InterfaceRequirement::new(id, VersionReq::parse(version)?)?);
        Ok(self)
    }

    pub fn require_worker(mut self) -> Self {
        self.require_worker = true;
        self
    }

    pub fn require_doctor(mut self) -> Self {
        self.require_doctor = true;
        self
    }

    pub fn require_cli_schema(mut self) -> Self {
        self.require_cli_schema = true;
        self
    }

    pub fn timeout(mut self, value: Duration) -> Self {
        self.timeout = value;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceReport {
    pub executable: PathBuf,
    pub manifest: Option<Manifest>,
    pub checks: Vec<DoctorCheck>,
}

impl ConformanceReport {
    pub fn is_healthy(&self) -> bool {
        self.checks
            .iter()
            .all(|check| check.status != DoctorStatus::Fail)
    }

    pub fn failures(&self) -> impl Iterator<Item = &DoctorCheck> {
        self.checks
            .iter()
            .filter(|check| check.status == DoctorStatus::Fail)
    }

    pub fn into_doctor_report(self) -> DoctorReport {
        DoctorReport::from_checks(self.checks)
    }

    pub fn assert_healthy(&self) {
        assert!(
            self.is_healthy(),
            "Outboard conformance failed:\n{}",
            self.checks
                .iter()
                .map(|check| format!("{:?} {}: {}", check.status, check.name, check.message))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

#[derive(Debug, Error)]
pub enum ConformanceError {
    #[error("invalid semver requirement: {0}")]
    Semver(#[from] semver::Error),
    #[error("invalid Outboard identifier: {0}")]
    Identifier(#[from] outboard::IdentifierError),
}

pub fn check_plugin(path: impl AsRef<Path>, options: &ConformanceOptions) -> ConformanceReport {
    let path = path.as_ref().to_path_buf();
    let mut checks = Vec::new();

    if !path.is_file() {
        checks.push(DoctorCheck::fail(
            "executable.exists",
            format!("{} is not a file", path.display()),
        ));
        return ConformanceReport {
            executable: path,
            manifest: None,
            checks,
        };
    }
    checks.push(DoctorCheck::pass(
        "executable.exists",
        path.display().to_string(),
    ));

    let output = match run_capture_args(&path, ["__outboard", "manifest"], None, options.timeout) {
        Ok(output) => output,
        Err(error) => {
            checks.push(DoctorCheck::fail("control.manifest", error.to_string()));
            return ConformanceReport {
                executable: path,
                manifest: None,
                checks,
            };
        }
    };
    if !output.status.success() {
        checks.push(DoctorCheck::fail(
            "control.manifest",
            format!(
                "exit {:?}; stderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
        return ConformanceReport {
            executable: path,
            manifest: None,
            checks,
        };
    }

    let manifest: Manifest = match serde_json::from_slice(&output.stdout) {
        Ok(manifest) => manifest,
        Err(error) => {
            checks.push(DoctorCheck::fail(
                "control.manifest.json",
                error.to_string(),
            ));
            return ConformanceReport {
                executable: path,
                manifest: None,
                checks,
            };
        }
    };
    checks.push(DoctorCheck::pass(
        "control.manifest",
        "returned valid manifest JSON",
    ));

    match manifest.validate() {
        Ok(()) => checks.push(DoctorCheck::pass("manifest.validate", "manifest is valid")),
        Err(error) => checks.push(DoctorCheck::fail("manifest.validate", error.to_string())),
    }

    if let Some(id) = &options.expected_id {
        if &manifest.plugin.id == id {
            checks.push(DoctorCheck::pass("manifest.identity", id.to_string()));
        } else {
            checks.push(DoctorCheck::fail(
                "manifest.identity",
                format!("expected {id}, got {}", manifest.plugin.id),
            ));
        }
    }

    for requirement in &options.required_interfaces {
        match manifest.interface(&requirement.id) {
            Some(interface) if requirement.version.matches(&interface.version) => {
                checks.push(DoctorCheck::pass(
                    format!("interface.{}", requirement.id),
                    format!("{} satisfies {}", interface.version, requirement.version),
                ));
            }
            Some(interface) => checks.push(DoctorCheck::fail(
                format!("interface.{}", requirement.id),
                format!(
                    "{} does not satisfy {}",
                    interface.version, requirement.version
                ),
            )),
            None => checks.push(DoctorCheck::fail(
                format!("interface.{}", requirement.id),
                "interface missing",
            )),
        }
    }

    check_doctor(&path, &manifest, options, &mut checks);
    check_cli_schema(&path, &manifest, options, &mut checks);
    check_control_ping(&path, &manifest, options, &mut checks);

    if options.require_worker || manifest.supports(ExecutionMode::Worker) {
        if !manifest.supports(ExecutionMode::Worker) {
            checks.push(DoctorCheck::fail(
                "worker.advertised",
                "worker was required but not advertised",
            ));
        } else {
            let plugin = ResolvedPlugin::from_parts(
                PluginCandidate::explicit(manifest.plugin.id.clone(), path.clone()),
                manifest.clone(),
            );
            match WorkerClient::spawn_with_interfaces_timeout(
                &plugin,
                options.required_interfaces.clone(),
                options.timeout,
            ) {
                Ok(mut worker) => {
                    checks.push(DoctorCheck::pass("worker.handshake", "handshake succeeded"));
                    match worker.ping(0xBADF00D) {
                        Ok(()) => checks.push(DoctorCheck::pass("worker.ping", "pong received")),
                        Err(error) => {
                            checks.push(DoctorCheck::fail("worker.ping", error.to_string()));
                        }
                    }
                    match worker.shutdown() {
                        Ok(()) => checks.push(DoctorCheck::pass(
                            "worker.shutdown",
                            "acknowledged and exited",
                        )),
                        Err(error) => {
                            checks.push(DoctorCheck::fail("worker.shutdown", error.to_string()))
                        }
                    }
                }
                Err(error) => {
                    checks.push(DoctorCheck::fail("worker.handshake", error.to_string()));
                }
            }
        }
    }

    ConformanceReport {
        executable: path,
        manifest: Some(manifest),
        checks,
    }
}

fn check_doctor(
    path: &Path,
    manifest: &Manifest,
    options: &ConformanceOptions,
    checks: &mut Vec<DoctorCheck>,
) {
    if options.require_doctor && !manifest.control.doctor {
        checks.push(DoctorCheck::fail(
            "control.doctor",
            "required operation not advertised",
        ));
        return;
    }
    if !manifest.control.doctor {
        return;
    }

    match run_capture_args(path, ["__outboard", "doctor"], None, options.timeout) {
        Ok(output) if output.status.success() => {
            match serde_json::from_slice::<DoctorReport>(&output.stdout) {
                Ok(report) if report.is_healthy() => checks.push(DoctorCheck::pass(
                    "control.doctor",
                    format!("healthy with {} checks", report.checks.len()),
                )),
                Ok(report) => checks.push(DoctorCheck::fail(
                    "control.doctor",
                    format!("plugin doctor reported {:?}", report.status),
                )),
                Err(error) => {
                    checks.push(DoctorCheck::fail("control.doctor.json", error.to_string()))
                }
            }
        }
        Ok(output) => checks.push(DoctorCheck::fail(
            "control.doctor",
            format!(
                "exit {:?}; stderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            ),
        )),
        Err(error) => checks.push(DoctorCheck::fail("control.doctor", error.to_string())),
    }
}

fn check_cli_schema(
    path: &Path,
    manifest: &Manifest,
    options: &ConformanceOptions,
    checks: &mut Vec<DoctorCheck>,
) {
    if options.require_cli_schema && !manifest.control.cli_schema {
        checks.push(DoctorCheck::fail(
            "control.cli-schema",
            "required operation not advertised",
        ));
        return;
    }
    if !manifest.control.cli_schema {
        return;
    }

    match run_capture_args(path, ["__outboard", "cli-schema"], None, options.timeout) {
        Ok(output) if output.status.success() => {
            match serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                Ok(value) if value.is_object() => checks.push(DoctorCheck::pass(
                    "control.cli-schema",
                    "returned a JSON object",
                )),
                Ok(_) => checks.push(DoctorCheck::fail(
                    "control.cli-schema.json",
                    "schema response was valid JSON but not an object",
                )),
                Err(error) => checks.push(DoctorCheck::fail(
                    "control.cli-schema.json",
                    error.to_string(),
                )),
            }
        }
        Ok(output) => checks.push(DoctorCheck::fail(
            "control.cli-schema",
            format!(
                "exit {:?}; stderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            ),
        )),
        Err(error) => checks.push(DoctorCheck::fail("control.cli-schema", error.to_string())),
    }
}

fn check_control_ping(
    path: &Path,
    manifest: &Manifest,
    options: &ConformanceOptions,
    checks: &mut Vec<DoctorCheck>,
) {
    if !manifest.control.ping {
        return;
    }
    match run_capture_args(path, ["__outboard", "ping"], None, options.timeout) {
        Ok(output) if output.status.success() => {
            match serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                Ok(value) if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) => {
                    checks.push(DoctorCheck::pass("control.ping", "plugin responded ok"));
                }
                Ok(_) => checks.push(DoctorCheck::fail(
                    "control.ping.json",
                    "ping response did not contain ok=true",
                )),
                Err(error) => {
                    checks.push(DoctorCheck::fail("control.ping.json", error.to_string()));
                }
            }
        }
        Ok(output) => checks.push(DoctorCheck::fail(
            "control.ping",
            format!(
                "exit {:?}; stderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            ),
        )),
        Err(error) => checks.push(DoctorCheck::fail("control.ping", error.to_string())),
    }
}
