use outboard_core::{DoctorReport, Manifest};
use serde_json::Value;
use std::{ffi::OsString, io::Write};
use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlRequest {
    Manifest,
    Doctor,
    CliSchema,
    Ping,
    Serve,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlDispatch {
    NotControl,
    Handled,
    Serve,
}
#[derive(Debug, Error)]
pub enum ControlError {
    #[error("unknown Outboard control command {0:?}")]
    UnknownCommand(OsString),
    #[error("Outboard control command is missing its operation")]
    MissingCommand,
    #[error("plugin does not expose a CLI schema")]
    NoCliSchema,
    #[error("failed to serialize control response: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to write control response: {0}")]
    Io(#[from] std::io::Error),
}
pub fn detect_control_request<I, S>(args: I) -> Result<Option<ControlRequest>, ControlError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut a = args.into_iter().map(Into::into);
    let Some(first) = a.next() else {
        return Ok(None);
    };
    if first.as_os_str() != std::ffi::OsStr::new("__outboard") {
        return Ok(None);
    }
    let Some(cmd) = a.next() else {
        return Err(ControlError::MissingCommand);
    };
    Ok(Some(match cmd.to_string_lossy().as_ref() {
        "manifest" => ControlRequest::Manifest,
        "doctor" => ControlRequest::Doctor,
        "cli-schema" => ControlRequest::CliSchema,
        "ping" => ControlRequest::Ping,
        "serve" => ControlRequest::Serve,
        _ => return Err(ControlError::UnknownCommand(cmd)),
    }))
}
pub struct ControlDispatcher<'a> {
    manifest: &'a Manifest,
    doctor: Option<Box<dyn Fn() -> DoctorReport + 'a>>,
    cli_schema: Option<Box<dyn Fn() -> Value + 'a>>,
}
impl<'a> ControlDispatcher<'a> {
    pub fn new(manifest: &'a Manifest) -> Self {
        Self {
            manifest,
            doctor: None,
            cli_schema: None,
        }
    }
    pub fn doctor(mut self, f: impl Fn() -> DoctorReport + 'a) -> Self {
        self.doctor = Some(Box::new(f));
        self
    }
    pub fn cli_schema(mut self, f: impl Fn() -> Value + 'a) -> Self {
        self.cli_schema = Some(Box::new(f));
        self
    }
    pub fn dispatch_from_env(self) -> Result<ControlDispatch, ControlError> {
        self.dispatch(std::env::args_os().skip(1))
    }
    pub fn dispatch<I, S>(self, args: I) -> Result<ControlDispatch, ControlError>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let Some(r) = detect_control_request(args)? else {
            return Ok(ControlDispatch::NotControl);
        };
        match r {
            ControlRequest::Serve => Ok(ControlDispatch::Serve),
            ControlRequest::Manifest => {
                write_json(self.manifest)?;
                Ok(ControlDispatch::Handled)
            }
            ControlRequest::Doctor => {
                write_json(&self.doctor.map_or_else(DoctorReport::healthy, |f| f()))?;
                Ok(ControlDispatch::Handled)
            }
            ControlRequest::CliSchema => {
                write_json(&self.cli_schema.ok_or(ControlError::NoCliSchema)?())?;
                Ok(ControlDispatch::Handled)
            }
            ControlRequest::Ping => {
                write_json(
                    &serde_json::json!({"ok":true,"plugin":self.manifest.plugin.id.to_string(),"version":self.manifest.plugin.version.to_string()}),
                )?;
                Ok(ControlDispatch::Handled)
            }
        }
    }
}
fn write_json(v: &impl serde::Serialize) -> Result<(), ControlError> {
    let stdout = std::io::stdout();
    let mut w = stdout.lock();
    serde_json::to_writer_pretty(&mut w, v)?;
    w.write_all(b"\n")?;
    w.flush()?;
    Ok(())
}
