use clap::{Args, Subcommand, ValueEnum};
use outboard::{
    DoctorReport, ExecutionMode, PluginCandidate, PluginName, Registry, ResolvedPlugin,
    WorkerClient, run_capture_args,
};
use serde::Serialize;
use std::ffi::OsString;
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}
#[derive(Debug, Args)]
pub struct PluginCommands {
    #[command(subcommand)]
    pub command: PluginCommand,
}
#[derive(Debug, Subcommand)]
pub enum PluginCommand {
    List(PluginListArgs),
    Inspect(PluginInspectArgs),
    Doctor(PluginDoctorArgs),
    Path(PluginPathArgs),
    Cache {
        #[command(subcommand)]
        command: PluginCacheCommand,
    },
}
#[derive(Debug, Args)]
pub struct PluginListArgs {
    #[arg(long)]
    pub kind: String,
    #[arg(long)]
    pub inspect: bool,
    #[arg(long)]
    pub all: bool,
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}
#[derive(Debug, Args)]
pub struct PluginInspectArgs {
    pub selector: String,
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}
#[derive(Debug, Args)]
pub struct PluginDoctorArgs {
    pub selector: String,
    #[arg(long)]
    pub deep: bool,
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}
#[derive(Debug, Args)]
pub struct PluginPathArgs {
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}
#[derive(Debug, Subcommand)]
pub enum PluginCacheCommand {
    Clear,
    Path,
}
pub fn execute_plugin_command(
    registry: &Registry,
    command: &PluginCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        PluginCommand::List(a) => list(registry, a),
        PluginCommand::Inspect(a) => inspect(registry, a),
        PluginCommand::Doctor(a) => doctor(registry, a),
        PluginCommand::Path(a) => path(registry, a),
        PluginCommand::Cache { command } => cache(registry, command),
    }
}
#[derive(Serialize)]
struct ListRow {
    id: String,
    path: String,
    source: String,
    selected: bool,
    version: Option<String>,
    inspect_error: Option<String>,
}
fn list(registry: &Registry, args: &PluginListArgs) -> Result<(), Box<dyn std::error::Error>> {
    let d = registry.discover(&args.kind)?;
    let mut rows = Vec::new();
    for c in &d.selected {
        rows.push(row(registry, c, true, args.inspect));
        if args.all {
            if let Some(s) = d.shadowed.get(&c.id) {
                rows.extend(s.iter().map(|c| row(registry, c, false, args.inspect)))
            }
        }
    }
    match args.format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
        OutputFormat::Text => {
            if rows.is_empty() {
                println!("no {} plugins discovered", args.kind)
            }
            for r in rows {
                let marker = if r.selected { "*" } else { " " };
                println!(
                    "{marker} {}{}\n    {} [{}]",
                    r.id,
                    r.version.map_or_else(String::new, |v| format!(" v{v}")),
                    r.path,
                    r.source
                );
                if let Some(e) = r.inspect_error {
                    println!("    inspect error: {e}")
                }
            }
        }
    }
    Ok(())
}
fn row(registry: &Registry, c: &PluginCandidate, selected: bool, do_inspect: bool) -> ListRow {
    let (version, inspect_error) = if do_inspect {
        match registry.inspect(c) {
            Ok(i) => (Some(i.manifest.plugin.version.to_string()), None),
            Err(e) => (None, Some(e.to_string())),
        }
    } else {
        (None, None)
    };
    ListRow {
        id: c.id.to_string(),
        path: c.path.to_string_lossy().into_owned(),
        source: format!("{:?}", c.source),
        selected,
        version,
        inspect_error,
    }
}
fn select(
    registry: &Registry,
    selector: &str,
) -> Result<(PluginCandidate, outboard::InspectedPlugin), Box<dyn std::error::Error>> {
    let (kind, name) = parse_selector(selector)?;
    let d = registry.discover(kind)?;
    let name = PluginName::new(name)?;
    let c = d
        .selected_by_name(&name)
        .ok_or_else(|| format!("plugin {selector} was not discovered"))?
        .clone();
    let i = registry.inspect(&c)?;
    Ok((c, i))
}
fn inspect(
    registry: &Registry,
    args: &PluginInspectArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let (c, i) = select(registry, &args.selector)?;
    match args.format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&i.manifest)?),
        OutputFormat::Text => {
            println!("{} v{}", i.manifest.plugin.id, i.manifest.plugin.version);
            println!("path: {}", c.path.display());
            println!("framework: {}", i.manifest.framework);
            println!("worker protocol: {}", i.manifest.worker_protocol);
            println!("execution: {:?}", i.manifest.execution);
            for x in i.manifest.interfaces {
                println!("interface: {}@{}", x.id, x.version)
            }
            for x in i.manifest.capabilities {
                println!("capability: {} {:?}", x.id, x.version)
            }
        }
    }
    Ok(())
}
#[derive(Serialize)]
struct DoctorOutput {
    plugin: String,
    report: DoctorReport,
    worker_handshake: Option<String>,
}
fn doctor(registry: &Registry, args: &PluginDoctorArgs) -> Result<(), Box<dyn std::error::Error>> {
    let (c, i) = select(registry, &args.selector)?;
    let o = run_capture_args(
        &c.path,
        [OsString::from("__outboard"), OsString::from("doctor")],
        None,
        registry.control_timeout(),
    )?;
    if !o.status.success() {
        return Err(format!("doctor failed: {}", String::from_utf8_lossy(&o.stderr)).into());
    }
    let report: DoctorReport = serde_json::from_slice(&o.stdout)?;
    let mut worker_handshake = None;
    if args.deep && i.manifest.supports(ExecutionMode::Worker) {
        let p = ResolvedPlugin::from_parts(c, i.manifest.clone());
        let mut w = WorkerClient::spawn(&p)?;
        w.ping(0xBADC0DE)?;
        w.shutdown()?;
        worker_handshake = Some("pass".into())
    }
    let v = DoctorOutput {
        plugin: i.manifest.plugin.id.to_string(),
        report,
        worker_handshake,
    };
    match args.format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&v)?),
        OutputFormat::Text => {
            println!("{}: {:?}", v.plugin, v.report.status);
            for c in &v.report.checks {
                println!("  {:?} {}: {}", c.status, c.name, c.message)
            }
            if let Some(w) = v.worker_handshake {
                println!("  worker handshake: {w}")
            }
        }
    }
    Ok(())
}
fn path(registry: &Registry, args: &PluginPathArgs) -> Result<(), Box<dyn std::error::Error>> {
    let roots = registry.search_roots();
    match args.format{OutputFormat::Json=>println!("{}",serde_json::to_string_pretty(&roots.iter().map(|r|serde_json::json!({"path":r.path.to_string_lossy(),"source":format!("{:?}",r.source)})).collect::<Vec<_>>())?),OutputFormat::Text=>for(i,r)in roots.iter().enumerate(){println!("{}: {} [{:?}]",i+1,r.path.display(),r.source)}}
    Ok(())
}
fn cache(registry: &Registry, cmd: &PluginCacheCommand) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        PluginCacheCommand::Clear => {
            registry.cache().clear();
            println!("manifest cache cleared")
        }
        PluginCacheCommand::Path => match registry.cache().path() {
            Some(p) => println!("{}", p.display()),
            None => println!("memory-only cache"),
        },
    }
    Ok(())
}
fn parse_selector(s: &str) -> Result<(&str, &str), Box<dyn std::error::Error>> {
    let Some((k, n)) = s.split_once(':') else {
        return Err("plugin selector must be KIND:NAME".into());
    };
    if k.is_empty() || n.is_empty() {
        return Err("plugin selector must be KIND:NAME".into());
    }
    Ok((k, n))
}
