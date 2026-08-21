//! Versioned worker protocol and lossless OS-string transport.
use outboard_core::{InterfaceId, InterfaceRequirement, Manifest};
use schemars::JsonSchema;
use semver::Version;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const DEFAULT_MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;
pub type RequestId = u64;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "encoding", content = "data", rename_all = "snake_case")]
pub enum WireOsString {
    Utf8(String),
    Unix(Vec<u8>),
    Windows(Vec<u16>),
}
impl WireOsString {
    pub fn from_os(v: &OsStr) -> Self {
        if let Some(s) = v.to_str() {
            return Self::Utf8(s.into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            return Self::Unix(v.as_bytes().to_vec());
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            return Self::Windows(v.encode_wide().collect());
        }
        #[cfg(not(any(unix, windows)))]
        {
            Self::Utf8(v.to_string_lossy().into_owned())
        }
    }
    pub fn into_os(self) -> Result<OsString, WireStringError> {
        match self {
            Self::Utf8(s) => Ok(s.into()),
            Self::Unix(b) => {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStringExt;
                    Ok(OsString::from_vec(b))
                }
                #[cfg(not(unix))]
                {
                    let _ = b;
                    Err(WireStringError::PlatformMismatch("unix"))
                }
            }
            Self::Windows(w) => {
                #[cfg(windows)]
                {
                    use std::os::windows::ffi::OsStringExt;
                    Ok(OsString::from_wide(&w))
                }
                #[cfg(not(windows))]
                {
                    let _ = w;
                    Err(WireStringError::PlatformMismatch("windows"))
                }
            }
        }
    }
}
impl From<&OsStr> for WireOsString {
    fn from(v: &OsStr) -> Self {
        Self::from_os(v)
    }
}
impl From<&OsString> for WireOsString {
    fn from(v: &OsString) -> Self {
        Self::from_os(v)
    }
}
impl From<OsString> for WireOsString {
    fn from(v: OsString) -> Self {
        Self::from_os(&v)
    }
}
impl From<&Path> for WireOsString {
    fn from(v: &Path) -> Self {
        Self::from_os(v.as_os_str())
    }
}
#[derive(Debug, Error)]
pub enum WireStringError {
    #[error("received a {0}-encoded OS string on a different platform")]
    PlatformMismatch(&'static str),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HostHello {
    pub framework: Version,
    pub protocol: Version,
    #[serde(default)]
    pub requested_interfaces: Vec<InterfaceRequirement>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PluginHello {
    pub protocol: Version,
    pub manifest: Manifest,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InvokeRequest {
    pub id: RequestId,
    pub interface: InterfaceId,
    pub command: String,
    #[serde(default)]
    pub args: Vec<WireOsString>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}
impl InvokeRequest {
    pub fn args_os(&self) -> Result<Vec<OsString>, WireStringError> {
        self.args
            .clone()
            .into_iter()
            .map(WireOsString::into_os)
            .collect()
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Payload {
    Json(Value),
    Text(String),
    Bytes(Vec<u8>),
    File(WireOsString),
}
impl Payload {
    pub fn file(p: impl AsRef<Path>) -> Self {
        Self::File(WireOsString::from(p.as_ref()))
    }
    pub fn into_file(self) -> Result<Option<PathBuf>, WireStringError> {
        match self {
            Self::File(v) => Ok(Some(PathBuf::from(v.into_os()?))),
            _ => Ok(None),
        }
    }
}
impl From<String> for Payload {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}
impl From<&str> for Payload {
    fn from(v: &str) -> Self {
        Self::Text(v.into())
    }
}
impl From<Value> for Payload {
    fn from(v: Value) -> Self {
        Self::Json(v)
    }
}
impl From<Vec<u8>> for Payload {
    fn from(v: Vec<u8>) -> Self {
        Self::Bytes(v)
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InvocationResult {
    pub status: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Payload>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}
impl InvocationResult {
    pub fn success(v: impl Into<Payload>) -> Self {
        Self {
            status: 0,
            result: Some(v.into()),
            metadata: BTreeMap::new(),
        }
    }
    pub fn empty_success() -> Self {
        Self {
            status: 0,
            result: None,
            metadata: BTreeMap::new(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}
impl WorkerError {
    pub fn new(c: impl Into<String>, m: impl Into<String>) -> Self {
        Self {
            code: c.into(),
            message: m.into(),
            details: None,
        }
    }
    pub fn details(mut self, v: impl Into<Value>) -> Self {
        self.details = Some(v.into());
        self
    }
}
impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for WorkerError {}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostFrame {
    Hello { hello: HostHello },
    Invoke { request: InvokeRequest },
    Cancel { id: RequestId },
    Ping { nonce: u64 },
    Shutdown,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginFrame {
    Hello {
        hello: PluginHello,
    },
    Started {
        id: RequestId,
    },
    Progress {
        id: RequestId,
        fraction: Option<f64>,
        message: Option<String>,
    },
    Output {
        id: RequestId,
        payload: Payload,
    },
    Finished {
        id: RequestId,
        result: InvocationResult,
    },
    Error {
        id: Option<RequestId>,
        error: WorkerError,
    },
    Pong {
        nonce: u64,
    },
    ShutdownAck,
}
impl PluginFrame {
    pub fn request_id(&self) -> Option<RequestId> {
        match self {
            Self::Started { id }
            | Self::Progress { id, .. }
            | Self::Output { id, .. }
            | Self::Finished { id, .. } => Some(*id),
            Self::Error { id, .. } => *id,
            Self::Hello { .. } | Self::Pong { .. } | Self::ShutdownAck => None,
        }
    }
}
pub fn host_frame_schema() -> schemars::Schema {
    schemars::schema_for!(HostFrame)
}
pub fn plugin_frame_schema() -> schemars::Schema {
    schemars::schema_for!(PluginFrame)
}
#[derive(Debug, Error)]
pub enum FrameError {
    #[error("I/O error while reading or writing worker frame: {0}")]
    Io(#[from] std::io::Error),
    #[error("worker frame length {length} exceeds configured maximum {max}")]
    TooLarge { length: usize, max: usize },
    #[error("invalid JSON worker frame: {0}")]
    Json(#[from] serde_json::Error),
    #[error("worker stream ended before a complete frame was read")]
    UnexpectedEof,
}
#[derive(Debug)]
pub struct FramedReader<R> {
    inner: R,
    max: usize,
}
impl<R: Read> FramedReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            max: DEFAULT_MAX_FRAME_SIZE,
        }
    }
    pub fn with_max_frame_size(inner: R, max: usize) -> Self {
        Self { inner, max }
    }
    pub fn read<T: DeserializeOwned>(&mut self) -> Result<T, FrameError> {
        let mut h = [0; 4];
        self.inner.read_exact(&mut h).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                FrameError::UnexpectedEof
            } else {
                FrameError::Io(e)
            }
        })?;
        let n = u32::from_be_bytes(h) as usize;
        if n > self.max {
            return Err(FrameError::TooLarge {
                length: n,
                max: self.max,
            });
        }
        let mut b = vec![0; n];
        self.inner.read_exact(&mut b).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                FrameError::UnexpectedEof
            } else {
                FrameError::Io(e)
            }
        })?;
        Ok(serde_json::from_slice(&b)?)
    }
}
#[derive(Debug)]
pub struct FramedWriter<W> {
    inner: W,
    max: usize,
}
impl<W: Write> FramedWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            max: DEFAULT_MAX_FRAME_SIZE,
        }
    }
    pub fn with_max_frame_size(inner: W, max: usize) -> Self {
        Self { inner, max }
    }
    pub fn write<T: Serialize>(&mut self, v: &T) -> Result<(), FrameError> {
        let b = serde_json::to_vec(v)?;
        if b.len() > self.max {
            return Err(FrameError::TooLarge {
                length: b.len(),
                max: self.max,
            });
        }
        let n = u32::try_from(b.len()).map_err(|_| FrameError::TooLarge {
            length: b.len(),
            max: u32::MAX as usize,
        })?;
        self.inner.write_all(&n.to_be_bytes())?;
        self.inner.write_all(&b)?;
        self.inner.flush()?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_roundtrip() {
        let mut bytes = Vec::new();
        FramedWriter::new(&mut bytes)
            .write(&HostFrame::Ping { nonce: 7 })
            .unwrap();
        let got: HostFrame = FramedReader::new(bytes.as_slice()).read().unwrap();
        assert_eq!(got, HostFrame::Ping { nonce: 7 });
    }
    #[cfg(unix)]
    #[test]
    fn non_utf8_roundtrip() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let src = OsString::from_vec(vec![b'a', 0xff, b'b']);
        let got = WireOsString::from(src.clone()).into_os().unwrap();
        assert_eq!(src.as_bytes(), got.as_bytes());
    }
}
