use clap::{Parser, Subcommand};
use outboard::{ExecutionMode, PluginRequirement, Registry, RegistryBuilder};
use outboard_clap::{PluginCommands, ToArgv, execute_plugin_command, validate_parser};
use outboard_demo_api::{
    DelayArgs, EchoArgs, EchoResult, EngineCli, EngineCommand, interface_id, interface_requirement,
};
use outboard_testing::{ConformanceOptions, check_plugin};
use outboard_tokio::WorkerClient as AsyncWorkerClient;
use std::{path::PathBuf, time::Duration};
#[derive(Debug, Parser)]
#[command(
    name = "outboard-demo",
    version,
    about = "End-to-end Outboard reference host"
)]
struct Cli {
    #[arg(long = "plugin-dir", global = true)]
    plugin_dirs: Vec<PathBuf>,
    #[arg(long, global = true)]
    no_path: bool,
    #[arg(long, global = true)]
    echo_override: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, Subcommand)]
enum Command {
    Plugins(PluginCommands),
    Echo(EchoArgs),
    WorkerEcho(EchoArgs),
    WorkerBatch,
    WorkerCancel,
    Conformance { executable: PathBuf },
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let registry = build_registry(&cli)?;
    match cli.command {
        Command::Plugins(c) => execute_plugin_command(&registry, &c.command)?,
        Command::Echo(a) => one_shot(&registry, a)?,
        Command::WorkerEcho(a) => worker_echo(&registry, a).await?,
        Command::WorkerBatch => worker_batch(&registry).await?,
        Command::WorkerCancel => worker_cancel(&registry).await?,
        Command::Conformance { executable } => conformance(executable)?,
    }
    Ok(())
}
fn build_registry(cli: &Cli) -> Result<Registry, Box<dyn std::error::Error>> {
    let mut b: RegistryBuilder = Registry::builder("outboard-demo")?.include_path(!cli.no_path);
    for p in &cli.plugin_dirs {
        b = b.plugin_path(p)
    }
    if let Some(p) = &cli.echo_override {
        b = b.override_plugin("engine", "echo", p)?
    }
    Ok(b.build())
}
fn requirement(worker: bool) -> Result<PluginRequirement, Box<dyn std::error::Error>> {
    let mut r = PluginRequirement::new("engine")?
        .named("echo")?
        .interface(interface_requirement());
    if worker {
        r = r.execution(ExecutionMode::Worker)
    }
    Ok(r)
}
fn one_shot(registry: &Registry, args: EchoArgs) -> Result<(), Box<dyn std::error::Error>> {
    let p = registry.resolve(&requirement(false)?)?;
    let cli = EngineCli {
        command: EngineCommand::Echo(args),
    };
    let argv = validate_parser(&cli)?;
    eprintln!("resolved {} -> {}", p.id(), p.path().display());
    eprintln!("typed argv: {:?}", argv);
    let o = p.run_capture(argv, None, Duration::from_secs(10))?;
    if !o.status.success() {
        return Err(format!("plugin failed: {}", String::from_utf8_lossy(&o.stderr)).into());
    }
    let result: EchoResult = serde_json::from_slice(&o.stdout)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
async fn worker_echo(
    registry: &Registry,
    args: EchoArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let p = registry.resolve(&requirement(true)?)?;
    let worker =
        AsyncWorkerClient::spawn_with_interfaces(&p, vec![interface_requirement()]).await?;
    let mut inv = worker
        .invoke(interface_id(), "echo", args.to_argv())
        .await?;
    while let Some(frame) = inv.next_event().await {
        println!("event: {}", serde_json::to_string(&frame)?);
        if matches!(
            frame,
            outboard::PluginFrame::Finished { .. } | outboard::PluginFrame::Error { .. }
        ) {
            break;
        }
    }
    worker.shutdown().await?;
    Ok(())
}
async fn worker_batch(registry: &Registry) -> Result<(), Box<dyn std::error::Error>> {
    let p = registry.resolve(&requirement(true)?)?;
    let worker =
        AsyncWorkerClient::spawn_with_interfaces(&p, vec![interface_requirement()]).await?;
    let first = worker
        .invoke(
            interface_id(),
            "delay",
            DelayArgs {
                milliseconds: 250,
                steps: 3,
                result: "first".into(),
            }
            .to_argv(),
        )
        .await?;
    let second = worker
        .invoke(
            interface_id(),
            "delay",
            DelayArgs {
                milliseconds: 120,
                steps: 2,
                result: "second".into(),
            }
            .to_argv(),
        )
        .await?;
    let (a, b) = tokio::join!(first.finish(), second.finish());
    let a = a?;
    let b = b?;
    println!("first: {}", serde_json::to_string_pretty(&a.result)?);
    println!("second: {}", serde_json::to_string_pretty(&b.result)?);
    let ap = pid(&a.result);
    let bp = pid(&b.result);
    if ap.is_none() || ap != bp {
        return Err("requests did not prove reuse of one worker process".into());
    }
    println!(
        "worker reuse verified: both requests ran in pid {}",
        ap.unwrap()
    );
    worker.shutdown().await?;
    Ok(())
}
fn pid(r: &outboard::InvocationResult) -> Option<u64> {
    match &r.result {
        Some(outboard::Payload::Json(v)) => v.get("process_id").and_then(serde_json::Value::as_u64),
        _ => None,
    }
}
async fn worker_cancel(registry: &Registry) -> Result<(), Box<dyn std::error::Error>> {
    let p = registry.resolve(&requirement(true)?)?;
    let worker =
        AsyncWorkerClient::spawn_with_interfaces(&p, vec![interface_requirement()]).await?;
    let inv = worker
        .invoke(
            interface_id(),
            "delay",
            DelayArgs {
                milliseconds: 5000,
                steps: 50,
                result: "should-not-finish".into(),
            }
            .to_argv(),
        )
        .await?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    inv.cancel().await?;
    match inv.finish().await {
        Err(e) if e.to_string().contains("cancelled") => println!("cancellation verified: {e}"),
        Err(e) => return Err(format!("unexpected error: {e}").into()),
        Ok(v) => {
            return Err(format!("cancelled request unexpectedly finished: {:?}", v.result).into());
        }
    }
    worker.shutdown().await?;
    Ok(())
}
fn conformance(executable: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let o = ConformanceOptions::default()
        .expected_id(outboard::PluginId::new("outboard-demo", "engine", "echo")?)
        .require_interface("demo.echo", "^1.0")?
        .require_worker()
        .require_doctor()
        .require_cli_schema();
    let r = check_plugin(executable, &o);
    println!("{}", serde_json::to_string_pretty(&r)?);
    if !r.is_healthy() {
        return Err("plugin failed conformance".into());
    }
    Ok(())
}
