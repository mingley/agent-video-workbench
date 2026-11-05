//! Bounded subprocess execution: owned process groups, cancellation, deadlines.
use crate::{Error, Result};
use nix::{
    sys::signal::{Signal, killpg},
    unistd::{Pid, getpid, getppid},
};
use std::{
    collections::VecDeque,
    io::Read,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

const LOG_LIMIT: usize = 64 * 1024;
const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

pub trait Control {
    fn check(&mut self) -> Result<()>;
    fn stage(&mut self, _stage: &str) -> Result<()> {
        self.check()
    }
}

pub struct Uncontrolled;
impl Control for Uncontrolled {
    fn check(&mut self) -> Result<()> {
        Ok(())
    }
}

pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: String,
}

fn capture(mut pipe: impl Read, tail: bool) -> std::io::Result<(Vec<u8>, bool)> {
    let limit = if tail { LOG_LIMIT } else { OUTPUT_LIMIT };
    let mut bytes = VecDeque::new();
    let mut buffer = [0_u8; 4096];
    let mut exceeded = false;
    loop {
        let count = pipe.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        for &byte in &buffer[..count] {
            if bytes.len() == limit {
                exceeded = true;
                if tail {
                    bytes.pop_front();
                } else {
                    continue;
                }
            }
            bytes.push_back(byte);
        }
    }
    Ok((bytes.into(), exceeded))
}

pub fn run(command: &mut Command, timeout: Duration, control: &mut dyn Control) -> Result<Output> {
    control.check()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let owner = getpid();
    // prctl runs between fork and exec. If the worker is killed, FFmpeg dies too.
    // The parent check closes the race where the worker died before prctl ran.
    unsafe {
        command.pre_exec(move || {
            nix::sys::resource::setrlimit(
                nix::sys::resource::Resource::RLIMIT_AS,
                4 * 1024 * 1024 * 1024,
                4 * 1024 * 1024 * 1024,
            )
            .map_err(std::io::Error::from)?;
            nix::sys::resource::setrlimit(
                nix::sys::resource::Resource::RLIMIT_FSIZE,
                32 * 1024 * 1024 * 1024,
                32 * 1024 * 1024 * 1024,
            )
            .map_err(std::io::Error::from)?;
            nix::sys::prctl::set_pdeathsig(Some(Signal::SIGKILL)).map_err(std::io::Error::from)?;
            if getppid() != owner {
                return Err(std::io::Error::other("execution owner exited"));
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    let group = Pid::from_raw(
        i32::try_from(child.id()).map_err(|_| Error::Invalid("process ID overflow".into()))?,
    );
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Invalid("stdout pipe missing".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Invalid("stderr pipe missing".into()))?;
    let out = thread::spawn(move || capture(stdout, false));
    let err = thread::spawn(move || capture(stderr, true));
    // Wake immediately when the child exits, while retaining the bounded
    // cancellation/lease checks for long-running media work. Polling try_wait
    // every 100ms imposed that delay on every short probe and thumbnail.
    let (completed, completion) = mpsc::sync_channel(1);
    let waiter = thread::spawn(move || {
        let _ = completed.send(child.wait());
    });
    let start = Instant::now();
    let outcome = loop {
        if let Err(error) = control.check() {
            break Err(error);
        }
        let Some(remaining) = timeout.checked_sub(start.elapsed()) else {
            break Err(Error::Timeout);
        };
        match completion.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(status) => break status.map_err(Error::from),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                break Err(Error::Invalid("subprocess waiter disconnected".into()));
            }
        }
    };
    // Kill descendants even if the direct child exited. They must not retain pipes.
    let _ = killpg(group, Signal::SIGKILL);
    waiter
        .join()
        .map_err(|_| Error::Invalid("subprocess waiter failed".into()))?;
    let (stdout, exceeded) = out
        .join()
        .map_err(|_| Error::Invalid("output reader failed".into()))??;
    let (stderr, _) = err
        .join()
        .map_err(|_| Error::Invalid("log reader failed".into()))??;
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
    let status = outcome?;
    if !status.success() {
        return Err(Error::Execution {
            message: format!("subprocess failed ({status})"),
            details: stderr,
        });
    }
    if exceeded {
        return Err(Error::Invalid(
            "subprocess output exceeds 8 MiB; request a smaller range".into(),
        ));
    }
    Ok(Output { stdout, stderr })
}
