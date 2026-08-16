use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{BufReader, BufWriter},
    process::{Child, ChildStdin, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread::{self, JoinHandle},
    time::Duration,
};

use outboard_core::{
    ExecutionMode, InterfaceId, InterfaceRequirement, Manifest, ResolvedPlugin,
    OUTBOARD_FRAMEWORK_VERSION, OUTBOARD_PROTOCOL_VERSION,
};
use outboard_protocol::{
    FrameError, FramedReader, FramedWriter, HostFrame, HostHello, InvocationResult,
    InvokeRequest, Payload, PluginFrame, PluginHello, RequestId, WireOsString, WorkerError,
};
use semver::Version;
use thiserror::Error;

pub const DEFAULT_CONTROL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum WorkerClientError {
    #[error("plugin does not advertise persistent worker execution")]
    WorkerNotSupported,
    #[error("failed to spawn plugin worker: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("worker framing error: {0}")]
    Frame(#[from] FrameError),
    #[error("worker sent {0:?} instead of hello")]
    ExpectedHello(PluginFrame),
    #[error("worker manifest changed between discovery and handshake")]
    ManifestChanged,
    #[error("worker protocol {actual} is not accepted by host requirement {accepted}")]
    Protocol {
        actual: Version,
        accepted: semver::VersionReq,
    },
    #[error("worker returned an error: {0}")]
    Worker(WorkerError),
    #[error("worker stream ended before invocation {0} completed")]
    Incomplete(RequestId),
    #[error("worker control operation timed out after {0:?}")]
    Timeout(Duration),
    #[error("worker reader channel closed unexpectedly")]
    ChannelClosed,
    #[error("failed waiting for worker child: {0}")]
    Wait(#[source] std::io::Error),
}

#[derive(Debug)]
pub struct InvocationTranscript {
    pub id: RequestId,
    pub events: Vec<PluginFrame>,
    pub result: InvocationResult,
}

pub struct WorkerClient {
    child: Child,
    reader_rx: Receiver<Result<PluginFrame, FrameError>>,
    reader_thread: Option<JoinHandle<()>>,
    writer: FramedWriter<BufWriter<ChildStdin>>,
    next_id: RequestId,
    manifest: Manifest,
    control_timeout: Duration,
}

impl WorkerClient {
    pub fn spawn(plugin: &ResolvedPlugin) -> Result<Self, WorkerClientError> {
        Self::spawn_with_interfaces(plugin, vec![])
    }

    pub fn spawn_with_interfaces(
        plugin: &ResolvedPlugin,
        requested_interfaces: Vec<InterfaceRequirement>,
    ) -> Result<Self, WorkerClientError> {
        Self::spawn_with_interfaces_timeout(
            plugin,
            requested_interfaces,
            DEFAULT_CONTROL_TIMEOUT,
        )
    }

    pub fn spawn_with_interfaces_timeout(
        plugin: &ResolvedPlugin,
        requested_interfaces: Vec<InterfaceRequirement>,
        control_timeout: Duration,
    ) -> Result<Self, WorkerClientError> {
        if !plugin.manifest.supports(ExecutionMode::Worker) {
            return Err(WorkerClientError::WorkerNotSupported);
        }

        let mut command = plugin.command();
        command
            .arg("__outboard")
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut child = command.spawn().map_err(WorkerClientError::Spawn)?;
        let stdin = child.stdin.take().expect("worker stdin was configured as piped");
        let stdout = child.stdout.take().expect("worker stdout was configured as piped");
        let mut writer = FramedWriter::new(BufWriter::new(stdin));

        let (reader_tx, reader_rx) = mpsc::channel();
        let reader_thread = thread::spawn(move || {
            let mut reader = FramedReader::new(BufReader::new(stdout));
            loop {
                let frame = reader.read::<PluginFrame>();
                let terminal = frame.is_err();
                if reader_tx.send(frame).is_err() || terminal {
                    break;
                }
            }
        });

        writer.write(&HostFrame::Hello {
            hello: HostHello {
                framework: Version::parse(OUTBOARD_FRAMEWORK_VERSION)
                    .expect("framework package version is valid semver"),
                protocol: Version::parse(OUTBOARD_PROTOCOL_VERSION)
                    .expect("Outboard protocol version is valid semver"),
                requested_interfaces,
                metadata: BTreeMap::new(),
            },
        })?;

        let frame = match recv_control(&reader_rx, control_timeout) {
            Ok(frame) => frame,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader_thread.join();
                return Err(error);
            }
        };
        let PluginFrame::Hello { hello } = frame else {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader_thread.join();
            return Err(WorkerClientError::ExpectedHello(frame));
        };
        if hello.manifest != plugin.manifest {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader_thread.join();
            return Err(WorkerClientError::ManifestChanged);
        }
        let host_protocol = semver::VersionReq::parse("^1.0")
            .expect("Outboard host protocol requirement is valid semver");
        if !host_protocol.matches(&hello.protocol) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader_thread.join();
            return Err(WorkerClientError::Protocol {
                actual: hello.protocol,
                accepted: host_protocol,
            });
        }

        Ok(Self {
            child,
            reader_rx,
            reader_thread: Some(reader_thread),
            writer,
            next_id: 1,
            manifest: hello.manifest,
            control_timeout,
        })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn control_timeout(&self) -> Duration {
        self.control_timeout
    }

    pub fn invoke(
        &mut self,
        interface: InterfaceId,
        command: impl Into<String>,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<InvocationTranscript, WorkerClientError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.writer.write(&HostFrame::Invoke {
            request: InvokeRequest {
                id,
                interface,
                command: command.into(),
                args: args.into_iter().map(WireOsString::from).collect(),
                metadata: BTreeMap::new(),
            },
        })?;

        let mut events = Vec::new();
        loop {
            let frame = match self.reader_rx.recv() {
                Ok(Ok(frame)) => frame,
                Ok(Err(FrameError::UnexpectedEof)) | Err(_) => {
                    return Err(WorkerClientError::Incomplete(id));
                }
                Ok(Err(error)) => return Err(WorkerClientError::Frame(error)),
            };
            match frame {
                PluginFrame::Finished { id: frame_id, result } if frame_id == id => {
                    events.push(PluginFrame::Finished {
                        id: frame_id,
                        result: result.clone(),
                    });
                    return Ok(InvocationTranscript { id, events, result });
                }
                PluginFrame::Error {
                    id: Some(frame_id),
                    error,
                } if frame_id == id => return Err(WorkerClientError::Worker(error)),
                other => events.push(other),
            }
        }
    }

    pub fn cancel(&mut self, id: RequestId) -> Result<(), WorkerClientError> {
        self.writer.write(&HostFrame::Cancel { id })?;
        Ok(())
    }

    pub fn ping(&mut self, nonce: u64) -> Result<(), WorkerClientError> {
        self.writer.write(&HostFrame::Ping { nonce })?;
        loop {
            match recv_control(&self.reader_rx, self.control_timeout)? {
                PluginFrame::Pong { nonce: received } if received == nonce => return Ok(()),
                PluginFrame::Error { error, .. } => return Err(WorkerClientError::Worker(error)),
                _ => {}
            }
        }
    }

    pub fn shutdown(mut self) -> Result<(), WorkerClientError> {
        self.writer.write(&HostFrame::Shutdown)?;
        loop {
            match recv_control(&self.reader_rx, self.control_timeout)? {
                PluginFrame::ShutdownAck => break,
                PluginFrame::Error { error, .. } => return Err(WorkerClientError::Worker(error)),
                _ => {}
            }
        }
        self.child.wait().map_err(WorkerClientError::Wait)?;
        if let Some(reader_thread) = self.reader_thread.take() {
            let _ = reader_thread.join();
        }
        Ok(())
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader_thread) = self.reader_thread.take() {
            let _ = reader_thread.join();
        }
    }
}

fn recv_control(
    receiver: &Receiver<Result<PluginFrame, FrameError>>,
    timeout: Duration,
) -> Result<PluginFrame, WorkerClientError> {
    match receiver.recv_timeout(timeout) {
        Ok(Ok(frame)) => Ok(frame),
        Ok(Err(error)) => Err(WorkerClientError::Frame(error)),
        Err(RecvTimeoutError::Timeout) => Err(WorkerClientError::Timeout(timeout)),
        Err(RecvTimeoutError::Disconnected) => Err(WorkerClientError::ChannelClosed),
    }
}

pub trait WorkerHandler {
    fn invoke(
        &mut self,
        request: InvokeRequest,
        responder: &mut SyncResponder<'_>,
    ) -> Result<InvocationResult, WorkerError>;

    fn cancel(&mut self, _id: RequestId) -> Result<(), WorkerError> {
        Ok(())
    }
}

trait FrameSink {
    fn send(&mut self, frame: &PluginFrame) -> Result<(), FrameError>;
}

impl<W: std::io::Write> FrameSink for FramedWriter<W> {
    fn send(&mut self, frame: &PluginFrame) -> Result<(), FrameError> {
        self.write(frame)
    }
}

pub struct SyncResponder<'a> {
    id: RequestId,
    writer: &'a mut dyn FrameSink,
}

impl SyncResponder<'_> {
    pub fn progress(
        &mut self,
        fraction: Option<f64>,
        message: Option<String>,
    ) -> Result<(), FrameError> {
        self.writer.send(&PluginFrame::Progress {
            id: self.id,
            fraction,
            message,
        })
    }

    pub fn output(&mut self, payload: impl Into<Payload>) -> Result<(), FrameError> {
        self.writer.send(&PluginFrame::Output {
            id: self.id,
            payload: payload.into(),
        })
    }
}

#[derive(Debug, Error)]
pub enum WorkerServerError {
    #[error("worker framing error: {0}")]
    Frame(#[from] FrameError),
    #[error("worker expected host hello as its first frame")]
    ExpectedHello,
    #[error("host framework {host} is not accepted by plugin requirement {accepted}")]
    Framework {
        host: Version,
        accepted: semver::VersionReq,
    },
    #[error("host worker protocol {host} is not accepted by plugin requirement {accepted}")]
    Protocol {
        host: Version,
        accepted: semver::VersionReq,
    },
    #[error("host requested incompatible interface: {0}")]
    Interface(String),
}

pub fn serve_worker<H: WorkerHandler>(
    manifest: Manifest,
    mut handler: H,
) -> Result<(), WorkerServerError> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = FramedReader::new(BufReader::new(stdin.lock()));
    let mut writer = FramedWriter::new(BufWriter::new(stdout.lock()));
    let first: HostFrame = reader.read()?;
    let HostFrame::Hello { hello } = first else {
        return Err(WorkerServerError::ExpectedHello);
    };
    validate_hello(&manifest, &hello)?;
    writer.write(&PluginFrame::Hello {
        hello: PluginHello {
            protocol: Version::parse(OUTBOARD_PROTOCOL_VERSION)
                .expect("Outboard protocol version is valid semver"),
            manifest: manifest.clone(),
        },
    })?;

    loop {
        match reader.read::<HostFrame>()? {
            HostFrame::Hello { .. } => writer.write(&PluginFrame::Error {
                id: None,
                error: WorkerError::new("duplicate_hello", "host hello may only be sent once"),
            })?,
            HostFrame::Ping { nonce } => writer.write(&PluginFrame::Pong { nonce })?,
            HostFrame::Shutdown => {
                writer.write(&PluginFrame::ShutdownAck)?;
                return Ok(());
            }
            HostFrame::Cancel { id } => {
                if let Err(error) = handler.cancel(id) {
                    writer.write(&PluginFrame::Error {
                        id: Some(id),
                        error,
                    })?;
                }
            }
            HostFrame::Invoke { request } => {
                let id = request.id;
                writer.write(&PluginFrame::Started { id })?;
                let mut responder = SyncResponder {
                    id,
                    writer: &mut writer,
                };
                match handler.invoke(request, &mut responder) {
                    Ok(result) => writer.write(&PluginFrame::Finished { id, result })?,
                    Err(error) => writer.write(&PluginFrame::Error {
                        id: Some(id),
                        error,
                    })?,
                }
            }
        }
    }
}

fn validate_hello(manifest: &Manifest, hello: &HostHello) -> Result<(), WorkerServerError> {
    if !manifest.framework.matches(&hello.framework) {
        return Err(WorkerServerError::Framework {
            host: hello.framework.clone(),
            accepted: manifest.framework.clone(),
        });
    }
    if !manifest.worker_protocol.matches(&hello.protocol) {
        return Err(WorkerServerError::Protocol {
            host: hello.protocol.clone(),
            accepted: manifest.worker_protocol.clone(),
        });
    }
    for requirement in &hello.requested_interfaces {
        let Some(interface) = manifest.interface(&requirement.id) else {
            return Err(WorkerServerError::Interface(format!(
                "missing {}",
                requirement.id
            )));
        };
        if !requirement.version.matches(&interface.version) {
            return Err(WorkerServerError::Interface(format!(
                "{} requires {}, plugin provides {}",
                requirement.id, requirement.version, interface.version
            )));
        }
    }
    Ok(())
}
