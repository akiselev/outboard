use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus { Pass, Warn, Fail }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DoctorCheck { pub name: String, pub status: DoctorStatus, pub message: String }
impl DoctorCheck {
    pub fn pass(name: impl Into<String>, message: impl Into<String>) -> Self { Self { name: name.into(), status: DoctorStatus::Pass, message: message.into() } }
    pub fn warn(name: impl Into<String>, message: impl Into<String>) -> Self { Self { name: name.into(), status: DoctorStatus::Warn, message: message.into() } }
    pub fn fail(name: impl Into<String>, message: impl Into<String>) -> Self { Self { name: name.into(), status: DoctorStatus::Fail, message: message.into() } }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DoctorReport { pub status: DoctorStatus, pub checks: Vec<DoctorCheck> }
impl DoctorReport {
    pub fn healthy() -> Self { Self::from_checks(vec![DoctorCheck::pass("plugin", "plugin reports healthy")]) }
    pub fn from_checks(checks: Vec<DoctorCheck>) -> Self {
        let status = if checks.iter().any(|c| c.status == DoctorStatus::Fail) { DoctorStatus::Fail }
            else if checks.iter().any(|c| c.status == DoctorStatus::Warn) { DoctorStatus::Warn } else { DoctorStatus::Pass };
        Self { status, checks }
    }
    pub fn is_healthy(&self) -> bool { self.status != DoctorStatus::Fail }
}
