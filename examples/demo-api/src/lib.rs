//! Shared typed interface used by the Outboard demo host and external plugin.
use clap::{Args, Parser, Subcommand, ValueEnum};
use outboard::{InterfaceId, InterfaceManifest, InterfaceRequirement};
use outboard_clap::ToArgv;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
pub const INTERFACE_ID: &str = "demo.echo";
pub const INTERFACE_VERSION: &str = "1.0.0";
pub fn interface_id() -> InterfaceId {
    InterfaceId::new(INTERFACE_ID).unwrap()
}
pub fn interface_manifest() -> InterfaceManifest {
    InterfaceManifest::new(INTERFACE_ID, Version::parse(INTERFACE_VERSION).unwrap()).unwrap()
}
pub fn interface_requirement() -> InterfaceRequirement {
    InterfaceRequirement::new(INTERFACE_ID, VersionReq::parse("^1.0").unwrap()).unwrap()
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum CaseMode {
    Keep,
    Upper,
    Lower,
}
impl Default for CaseMode {
    fn default() -> Self {
        Self::Keep
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Args, ToArgv)]
pub struct EchoArgs {
    pub message: String,
    #[arg(long, default_value_t = 1)]
    pub repeat: usize,
    #[arg(long, value_enum, default_value = "keep")]
    pub case: CaseMode,
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    #[arg(long)]
    pub prefix: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Args, ToArgv)]
pub struct DelayArgs {
    #[arg(long, default_value_t = 500)]
    pub milliseconds: u64,
    #[arg(long, default_value_t = 5)]
    pub steps: u32,
    #[arg(long, default_value = "done")]
    pub result: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Subcommand, ToArgv)]
#[command(rename_all = "kebab-case")]
pub enum EngineCommand {
    Echo(EchoArgs),
    Delay(DelayArgs),
}
#[derive(Debug, Clone, PartialEq, Eq, Parser, ToArgv)]
#[command(
    name = "demo-engine",
    version,
    about = "Outboard demo engine interface"
)]
pub struct EngineCli {
    #[command(subcommand)]
    pub command: EngineCommand,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchoResult {
    pub text: String,
    pub tags: Vec<String>,
    pub process_id: u32,
}
pub fn run_echo(args: &EchoArgs) -> EchoResult {
    let text = match args.case {
        CaseMode::Keep => args.message.clone(),
        CaseMode::Upper => args.message.to_uppercase(),
        CaseMode::Lower => args.message.to_lowercase(),
    };
    let repeated = (0..args.repeat)
        .map(|_| text.clone())
        .collect::<Vec<_>>()
        .join(" ");
    let text = args
        .prefix
        .as_ref()
        .map_or(repeated.clone(), |p| format!("{p}{repeated}"));
    EchoResult {
        text,
        tags: args.tags.clone(),
        process_id: std::process::id(),
    }
}
