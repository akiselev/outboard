//! Outboard: typed, discoverable, versioned external-process extensions.
mod control;mod worker;
pub use outboard_core::*;pub use outboard_protocol::*;pub use outboard_protocol as protocol;
pub use control::{ControlDispatch,ControlDispatcher,ControlError,ControlRequest,detect_control_request};
pub use worker::{InvocationTranscript,SyncResponder,WorkerClient,WorkerClientError,WorkerHandler,WorkerServerError,DEFAULT_CONTROL_TIMEOUT,serve_worker};
