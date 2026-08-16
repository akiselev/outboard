//! Tokio integration for concurrent, cancellable persistent Outboard workers.

use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use outboard::{
    ExecutionMode, InterfaceId, InterfaceRequirement, Manifest, ResolvedPlugin,
    DEFAULT_CONTROL_TIMEOUT, OUTBOARD_FRAMEWORK_VERSION, OUTBOARD_PROTOCOL_VERSION,
};
use outboard_protocol::{
    DEFAULT_MAX_FRAME_SIZE, FrameError, HostFrame, HostHello, InvocationResult, InvokeRequest,
    Payload, PluginFrame, PluginHello, RequestId, WireOsString, WorkerError,
};
use semver::Version;
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, Mutex},
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

pub struct AsyncFramedReader<R> {
    inner: R,
    max: usize,
}

impl<R: AsyncRead + Unpin> AsyncFramedReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            max: DEFAULT_MAX_FRAME_SIZE,
        }
    }

    pub fn with_max_frame_size(inner: R, max: usize) -> Self {
        Self { inner, max }
    }

    pub async fn read<T: DeserializeOwned>(&mut self) -> Result<T, FrameError> {
        let mut header = [0; 4];
        self.inner.read_exact(&mut header).await.map_err(map_read)?;
        let length = u32::from_be_bytes(header) as usize;
        if length > self.max {
            return Err(FrameError::TooLarge {
                length,
                max: self.max,
            });
        }
        let mut bytes = vec![0; length];
        self.inner.read_exact(&mut bytes).await.map_err(map_read)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

fn map_read(error: std::io::Error) -> FrameError {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        FrameError::UnexpectedEof
    } else {
        FrameError::Io(error)
    }
}

pub struct AsyncFramedWriter<W> {
    inner: W,
    max: usize,
}

impl<W: AsyncWrite + Unpin> AsyncFramedWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            max: DEFAULT_MAX_FRAME_SIZE,
        }
    }

    pub fn with_max_frame_size(inner: W, max: usize) -> Self {
        Self { inner, max }
    }

    pub async fn write<T: Serialize>(&mut self, value: &T) -> Result<(), FrameError> {
        let bytes = serde_json::to_vec(value)?;
        if bytes.len() > self.max {
            return Err(FrameError::TooLarge {
                length: bytes.len(),
                max: self.max,
            });
        }
        let length = u32::try_from(bytes.len()).map_err(|_| FrameError::TooLarge {
            length: bytes.len(),
            max: u32::MAX as usize,
        })?;
        self.inner.write_all(&length.to_be_bytes()).await?;
        self.inner.write_all(&bytes).await?;
        self.inner.flush().await?;
        Ok(())
    }
}

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
    #[error("worker returned {0}")]
    Worker(WorkerError),
    #[error("worker response channel closed for invocation {0}")]
    ChannelClosed(RequestId),
    #[error("worker control operation timed out after {0:?}")]
    Timeout(Duration),
    #[error("worker process wait failed: {0}")]
    Wait(#[source] std::io::Error),
}

struct ClientInner {
    writer: Mutex<AsyncFramedWriter<BufWriter<ChildStdin>>>,
    pending: Mutex<HashMap<RequestId, mpsc::UnboundedSender<PluginFrame>>>,
    next_id: AtomicU64,
}

pub struct WorkerClient {
    child: Child,
    inner: Arc<ClientInner>,
    global_rx: mpsc::UnboundedReceiver<PluginFrame>,
    reader_task: JoinHandle<()>,
    manifest: Manifest,
    control_timeout: Duration,
}

impl WorkerClient {
    pub async fn spawn(plugin: &ResolvedPlugin) -> Result<Self, WorkerClientError> {
        Self::spawn_with_interfaces(plugin, vec![]).await
    }

    pub async fn spawn_with_interfaces(
        plugin: &ResolvedPlugin,
        requested_interfaces: Vec<InterfaceRequirement>,
    ) -> Result<Self, WorkerClientError> {
        Self::spawn_with_interfaces_timeout(
            plugin,
            requested_interfaces,
            DEFAULT_CONTROL_TIMEOUT,
        )
        .await
    }

    pub async fn spawn_with_interfaces_timeout(
        plugin: &ResolvedPlugin,
        requested_interfaces: Vec<InterfaceRequirement>,
        control_timeout: Duration,
    ) -> Result<Self, WorkerClientError> {
        if !plugin.manifest.supports(ExecutionMode::Worker) {
            return Err(WorkerClientError::WorkerNotSupported);
        }

        let mut command = Command::new(plugin.path());
        command
            .arg("__outboard")
            .arg("serve")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(WorkerClientError::Spawn)?;
        let stdin = child.stdin.take().expect("worker stdin was configured as piped");
        let stdout = child.stdout.take().expect("worker stdout was configured as piped");
        let mut writer = AsyncFramedWriter::new(BufWriter::new(stdin));
        let mut reader = AsyncFramedReader::new(BufReader::new(stdout));

        writer
            .write(&HostFrame::Hello {
                hello: HostHello {
                    framework: Version::parse(OUTBOARD_FRAMEWORK_VERSION)
                        .expect("framework package version is valid semver"),
                    protocol: Version::parse(OUTBOARD_PROTOCOL_VERSION)
                        .expect("Outboard protocol version is valid semver"),
                    requested_interfaces,
                    metadata: BTreeMap::new(),
                },
            })
            .await?;

        let frame: PluginFrame = tokio::time::timeout(control_timeout, reader.read())
            .await
            .map_err(|_| WorkerClientError::Timeout(control_timeout))??;
        let PluginFrame::Hello { hello } = frame else {
            return Err(WorkerClientError::ExpectedHello(frame));
        };
        if hello.manifest != plugin.manifest {
            return Err(WorkerClientError::ManifestChanged);
        }
        let host_protocol = semver::VersionReq::parse("^1.0")
            .expect("Outboard host protocol requirement is valid semver");
        if !host_protocol.matches(&hello.protocol) {
            return Err(WorkerClientError::Protocol {
                actual: hello.protocol,
                accepted: host_protocol,
            });
        }

        let inner = Arc::new(ClientInner {
            writer: Mutex::new(writer),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        });
        let (global_tx, global_rx) = mpsc::unbounded_channel();
        let reader_inner = Arc::clone(&inner);
        let reader_task = tokio::spawn(async move {
            loop {
                let frame = match reader.read::<PluginFrame>().await {
                    Ok(frame) => frame,
                    Err(_) => break,
                };
                if let Some(id) = frame.request_id() {
                    let terminal = matches!(
                        &frame,
                        PluginFrame::Finished { .. } | PluginFrame::Error { .. }
                    );
                    let tx = { reader_inner.pending.lock().await.get(&id).cloned() };
                    if let Some(tx) = tx {
                        let _ = tx.send(frame);
                    }
                    if terminal {
                        reader_inner.pending.lock().await.remove(&id);
                    }
                } else if global_tx.send(frame).is_err() {
                    break;
                }
            }
            reader_inner.pending.lock().await.clear();
        });

        Ok(Self {
            child,
            inner,
            global_rx,
            reader_task,
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

    pub async fn invoke(
        &self,
        interface: InterfaceId,
        command: impl Into<String>,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Invocation, WorkerClientError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        self.inner.pending.lock().await.insert(id, tx);
        let request = InvokeRequest {
            id,
            interface,
            command: command.into(),
            args: args.into_iter().map(WireOsString::from).collect(),
            metadata: BTreeMap::new(),
        };
        if let Err(error) = self
            .inner
            .writer
            .lock()
            .await
            .write(&HostFrame::Invoke { request })
            .await
        {
            self.inner.pending.lock().await.remove(&id);
            return Err(error.into());
        }
        Ok(Invocation {
            id,
            inner: Arc::clone(&self.inner),
            rx,
            events: vec![],
        })
    }

    pub async fn ping(&mut self, nonce: u64) -> Result<(), WorkerClientError> {
        self.inner
            .writer
            .lock()
            .await
            .write(&HostFrame::Ping { nonce })
            .await?;
        let timeout = self.control_timeout;
        tokio::time::timeout(timeout, async {
            while let Some(frame) = self.global_rx.recv().await {
                match frame {
                    PluginFrame::Pong { nonce: received } if received == nonce => return Ok(()),
                    PluginFrame::Error { error, .. } => {
                        return Err(WorkerClientError::Worker(error));
                    }
                    _ => {}
                }
            }
            Err(WorkerClientError::ChannelClosed(0))
        })
        .await
        .map_err(|_| WorkerClientError::Timeout(timeout))?
    }

    pub async fn shutdown(mut self) -> Result<(), WorkerClientError> {
        self.inner
            .writer
            .lock()
            .await
            .write(&HostFrame::Shutdown)
            .await?;
        let timeout = self.control_timeout;
        tokio::time::timeout(timeout, async {
            while let Some(frame) = self.global_rx.recv().await {
                match frame {
                    PluginFrame::ShutdownAck => return Ok(()),
                    PluginFrame::Error { error, .. } => {
                        return Err(WorkerClientError::Worker(error));
                    }
                    _ => {}
                }
            }
            Err(WorkerClientError::ChannelClosed(0))
        })
        .await
        .map_err(|_| WorkerClientError::Timeout(timeout))??;

        match tokio::time::timeout(timeout, self.child.wait()).await {
            Ok(result) => {
                result.map_err(WorkerClientError::Wait)?;
            }
            Err(_) => {
                let _ = self.child.start_kill();
                let _ = self.child.wait().await;
                return Err(WorkerClientError::Timeout(timeout));
            }
        }
        self.reader_task.abort();
        Ok(())
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

pub struct Invocation {
    id: RequestId,
    inner: Arc<ClientInner>,
    rx: mpsc::UnboundedReceiver<PluginFrame>,
    events: Vec<PluginFrame>,
}

impl Invocation {
    pub fn id(&self) -> RequestId {
        self.id
    }

    pub async fn cancel(&self) -> Result<(), WorkerClientError> {
        self.inner
            .writer
            .lock()
            .await
            .write(&HostFrame::Cancel { id: self.id })
            .await?;
        Ok(())
    }

    pub async fn next_event(&mut self) -> Option<PluginFrame> {
        self.rx.recv().await
    }

    pub async fn finish(mut self) -> Result<AsyncInvocationTranscript, WorkerClientError> {
        while let Some(frame) = self.rx.recv().await {
            match frame {
                PluginFrame::Finished { id, result } if id == self.id => {
                    self.events.push(PluginFrame::Finished {
                        id,
                        result: result.clone(),
                    });
                    return Ok(AsyncInvocationTranscript {
                        id,
                        events: self.events,
                        result,
                    });
                }
                PluginFrame::Error {
                    id: Some(id),
                    error,
                } if id == self.id => return Err(WorkerClientError::Worker(error)),
                other => self.events.push(other),
            }
        }
        Err(WorkerClientError::ChannelClosed(self.id))
    }
}

#[derive(Debug)]
pub struct AsyncInvocationTranscript {
    pub id: RequestId,
    pub events: Vec<PluginFrame>,
    pub result: InvocationResult,
}

#[derive(Clone)]
pub struct InvocationContext {
    id: RequestId,
    cancellation: CancellationToken,
    tx: mpsc::UnboundedSender<PluginFrame>,
}

impl InvocationContext {
    pub fn id(&self) -> RequestId {
        self.id
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }

    pub fn progress(
        &self,
        fraction: Option<f64>,
        message: Option<String>,
    ) -> Result<(), WorkerError> {
        self.tx
            .send(PluginFrame::Progress {
                id: self.id,
                fraction,
                message,
            })
            .map_err(|_| WorkerError::new("transport_closed", "worker output channel is closed"))
    }

    pub fn output(&self, payload: impl Into<Payload>) -> Result<(), WorkerError> {
        self.tx
            .send(PluginFrame::Output {
                id: self.id,
                payload: payload.into(),
            })
            .map_err(|_| WorkerError::new("transport_closed", "worker output channel is closed"))
    }
}

#[async_trait]
pub trait AsyncWorkerHandler: Send + Sync + 'static {
    async fn invoke(
        &self,
        context: InvocationContext,
        request: InvokeRequest,
    ) -> Result<InvocationResult, WorkerError>;

    async fn cancel(&self, _id: RequestId) -> Result<(), WorkerError> {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum WorkerServerError {
    #[error("worker framing error: {0}")]
    Frame(#[from] FrameError),
    #[error("worker expected host hello as first frame")]
    ExpectedHello,
    #[error("host framework {host} is incompatible with {accepted}")]
    Framework {
        host: Version,
        accepted: semver::VersionReq,
    },
    #[error("host worker protocol {host} is incompatible with {accepted}")]
    Protocol {
        host: Version,
        accepted: semver::VersionReq,
    },
    #[error("host requested incompatible interface: {0}")]
    Interface(String),
    #[error("writer task failed: {0}")]
    WriterJoin(#[source] tokio::task::JoinError),
}

pub async fn serve_worker<H: AsyncWorkerHandler>(
    manifest: Manifest,
    handler: Arc<H>,
) -> Result<(), WorkerServerError> {
    let mut reader = AsyncFramedReader::new(BufReader::new(tokio::io::stdin()));
    let mut initial_writer = AsyncFramedWriter::new(BufWriter::new(tokio::io::stdout()));
    let first: HostFrame = reader.read().await?;
    let HostFrame::Hello { hello } = first else {
        return Err(WorkerServerError::ExpectedHello);
    };
    validate_hello(&manifest, &hello)?;
    initial_writer
        .write(&PluginFrame::Hello {
            hello: PluginHello {
                protocol: Version::parse(OUTBOARD_PROTOCOL_VERSION)
                    .expect("Outboard protocol version is valid semver"),
                manifest: manifest.clone(),
            },
        })
        .await?;

    let (tx, mut rx) = mpsc::unbounded_channel::<PluginFrame>();
    let writer_task = tokio::spawn(async move {
        let mut writer = initial_writer;
        while let Some(frame) = rx.recv().await {
            writer.write(&frame).await?;
        }
        Ok::<(), FrameError>(())
    });
    let pending: Arc<Mutex<HashMap<RequestId, CancellationToken>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let mut tasks = JoinSet::new();

    loop {
        while tasks.try_join_next().is_some() {}
        let frame = reader.read::<HostFrame>().await?;
        match frame {
            HostFrame::Hello { .. } => {
                let _ = tx.send(PluginFrame::Error {
                    id: None,
                    error: WorkerError::new("duplicate_hello", "hello may only be sent once"),
                });
            }
            HostFrame::Ping { nonce } => {
                let _ = tx.send(PluginFrame::Pong { nonce });
            }
            HostFrame::Cancel { id } => {
                if let Some(token) = pending.lock().await.get(&id).cloned() {
                    token.cancel();
                }
                if let Err(error) = handler.cancel(id).await {
                    let _ = tx.send(PluginFrame::Error {
                        id: Some(id),
                        error,
                    });
                }
            }
            HostFrame::Shutdown => {
                for token in pending.lock().await.values() {
                    token.cancel();
                }
                while tasks.join_next().await.is_some() {}
                let _ = tx.send(PluginFrame::ShutdownAck);
                break;
            }
            HostFrame::Invoke { request } => {
                let id = request.id;
                let mut map = pending.lock().await;
                if map.contains_key(&id) {
                    drop(map);
                    let _ = tx.send(PluginFrame::Error {
                        id: Some(id),
                        error: WorkerError::new(
                            "duplicate_request",
                            "request id is already active",
                        ),
                    });
                    continue;
                }
                let token = CancellationToken::new();
                map.insert(id, token.clone());
                drop(map);

                let _ = tx.send(PluginFrame::Started { id });
                let handler = Arc::clone(&handler);
                let task_tx = tx.clone();
                let task_pending = Arc::clone(&pending);
                tasks.spawn(async move {
                    let context = InvocationContext {
                        id,
                        cancellation: token,
                        tx: task_tx.clone(),
                    };
                    let frame = match handler.invoke(context, request).await {
                        Ok(result) => PluginFrame::Finished { id, result },
                        Err(error) => PluginFrame::Error {
                            id: Some(id),
                            error,
                        },
                    };
                    let _ = task_tx.send(frame);
                    task_pending.lock().await.remove(&id);
                });
            }
        }
    }

    drop(tx);
    writer_task.await.map_err(WorkerServerError::WriterJoin)??;
    Ok(())
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

pub trait TokioPluginExt {
    fn tokio_command(&self, args: impl IntoIterator<Item = OsString>) -> Command;
}

impl TokioPluginExt for ResolvedPlugin {
    fn tokio_command(&self, args: impl IntoIterator<Item = OsString>) -> Command {
        let mut command = Command::new(self.path());
        command.args(args);
        command
    }
}
