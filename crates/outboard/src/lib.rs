//! Outboard: typed, discoverable, versioned external-process extensions.
mod control;
mod worker;
pub use control::{
    ControlDispatch, ControlDispatcher, ControlError, ControlRequest, detect_control_request,
};
pub use outboard_core::*;
pub use outboard_protocol as protocol;
pub use outboard_protocol::*;
pub use worker::{
    DEFAULT_CONTROL_TIMEOUT, InvocationTranscript, SyncResponder, WorkerClient, WorkerClientError,
    WorkerHandler, WorkerServerError, serve_worker,
};
