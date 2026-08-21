//! Core primitives for Outboard, a Cargo-style external executable plugin framework.

mod cache;
mod discovery;
mod doctor;
mod id;
mod manifest;
mod process;
mod registry;

pub use cache::{CacheMode, ManifestCache};
pub use discovery::{
    DiscoverySet, DiscoverySource, PluginCandidate, SearchRoot, executable_file_name,
};
pub use doctor::{DoctorCheck, DoctorReport, DoctorStatus};
pub use id::{
    CapabilityId, IdentifierError, InterfaceId, Namespace, PluginId, PluginKind, PluginName,
};
pub use manifest::{
    Capability, CapabilityRequirement, CompatibilityIssue, ControlCapabilities, ExecutionMode,
    InterfaceManifest, InterfaceRequirement, MANIFEST_VERSION, Manifest, ManifestBuilder,
    ManifestError, OUTBOARD_FRAMEWORK_VERSION, OUTBOARD_PROTOCOL_VERSION, PluginMetadata,
    PluginRequirement,
};
pub use process::{CapturedOutput, ProcessError, run_capture, run_capture_args};
pub use registry::{
    CandidateRejection, InspectedPlugin, Registry, RegistryBuilder, RegistryError, ResolvedPlugin,
    inspect_manifest_at,
};
