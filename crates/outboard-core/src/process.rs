use std::{
    ffi::OsString,
    io::{Read, Write},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use thiserror::Error;

#[derive(Debug)]
pub struct CapturedOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("failed to spawn process: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("failed while waiting for process: {0}")]
    Wait(#[source] std::io::Error),
    #[error("process exceeded timeout of {duration:?}")]
    Timeout {
        duration: Duration,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    #[error("failed to join process I/O thread")]
    ReaderPanicked,
    #[error("failed to write process stdin: {0}")]
    Stdin(#[source] std::io::Error),
}

impl ProcessError {
    pub fn captured_stdout(&self) -> Option<&[u8]> {
        match self {
            Self::Timeout { stdout, .. } => Some(stdout),
            _ => None,
        }
    }

    pub fn captured_stderr(&self) -> Option<&[u8]> {
        match self {
            Self::Timeout { stderr, .. } => Some(stderr),
            _ => None,
        }
    }
}

pub fn run_capture_args<I, S>(
    program: impl AsRef<std::ffi::OsStr>,
    args: I,
    input: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<CapturedOutput, ProcessError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut command = Command::new(program);
    command.args(args.into_iter().map(Into::into));
    run_capture(&mut command, input, timeout)
}

pub fn run_capture(
    command: &mut Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<CapturedOutput, ProcessError> {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });

    let mut child = command.spawn().map_err(ProcessError::Spawn)?;
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let mut stderr = child.stderr.take().expect("stderr was piped");

    // Drain both pipes concurrently. Waiting on the child before draining can deadlock if the
    // plugin fills either OS pipe buffer.
    let stdout_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stdin_thread = input.map(|bytes| {
        let mut stdin = child.stdin.take().expect("stdin was piped");
        thread::spawn(move || {
            stdin.write_all(&bytes)?;
            stdin.flush()
        })
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait().map_err(ProcessError::Wait)? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();

                if let Some(thread) = stdin_thread {
                    let _ = thread.join();
                }
                let captured_stdout = stdout_thread
                    .join()
                    .map_err(|_| ProcessError::ReaderPanicked)?
                    .map_err(ProcessError::Wait)?;
                let captured_stderr = stderr_thread
                    .join()
                    .map_err(|_| ProcessError::ReaderPanicked)?
                    .map_err(ProcessError::Wait)?;

                return Err(ProcessError::Timeout {
                    duration: timeout,
                    stdout: captured_stdout,
                    stderr: captured_stderr,
                });
            }
            None => thread::sleep(Duration::from_millis(5)),
        }
    };

    if let Some(thread) = stdin_thread {
        thread
            .join()
            .map_err(|_| ProcessError::ReaderPanicked)?
            .map_err(ProcessError::Stdin)?;
    }
    let stdout = stdout_thread
        .join()
        .map_err(|_| ProcessError::ReaderPanicked)?
        .map_err(ProcessError::Wait)?;
    let stderr = stderr_thread
        .join()
        .map_err(|_| ProcessError::ReaderPanicked)?
        .map_err(ProcessError::Wait)?;

    Ok(CapturedOutput {
        status,
        stdout,
        stderr,
    })
}
