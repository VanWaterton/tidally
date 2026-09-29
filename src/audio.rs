//! Where audio goes: this machine's speakers, or (in the experimental `remote` edition) a
//! PulseAudio/PipeWire server forwarded from an SSH client.
//!
//! Over SSH, the client forwards its sound server's socket to a unique
//! `$XDG_RUNTIME_DIR/pulse-ssh-*.sock` path on this machine:
//!
//! ```sh
//! ssh -R /run/user/<uid>/pulse-ssh-$$.sock:$XDG_RUNTIME_DIR/pulse/native host
//! ```
//!
//! and `tidally-remote` picks it up. Setting `PULSE_SERVER` explicitly also works.

#[cfg(feature = "remote")]
use std::os::unix::net::UnixStream;
#[cfg(feature = "remote")]
use std::path::PathBuf;
#[cfg(feature = "remote")]
use std::process::{Command, Stdio};
#[cfg(feature = "remote")]
use std::time::{Duration, Instant};

#[cfg(feature = "remote")]
/// Forwarded sockets are named `pulse-ssh*.sock` in `$XDG_RUNTIME_DIR`.
const SSH_SOCKET_PREFIX: &str = "pulse-ssh";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// The local default output.
    Local,
    #[cfg(feature = "remote")]
    /// Local output, but we're in an SSH session with no forwarded audio. Worth warning about,
    /// since the music will come out of the remote machine's speakers.
    LocalOverSsh,
    #[cfg(feature = "remote")]
    /// A PulseAudio-protocol server, e.g. `unix:/run/user/1000/pulse-ssh-123.sock`.
    Pulse(String),
    #[cfg(feature = "remote")]
    /// A forwarded server that doesn't answer: the SSH link is up, but nothing on the client end
    /// is serving sound. We still send audio there (rather than surprise-playing on this
    /// machine's speakers), and warn.
    Unreachable(String),
}

impl Route {
    #[cfg(feature = "remote")]
    pub fn detect() -> Self {
        let server = match std::env::var("PULSE_SERVER") {
            Ok(server) if !server.is_empty() => server,
            _ if std::env::var_os("SSH_CONNECTION").is_none() => return Route::Local,
            _ => match forwarded_socket() {
                Some(path) => format!("unix:{}", path.display()),
                None => return Route::LocalOverSsh,
            },
        };
        // SSH accepts connections on a forwarded socket even when nothing is listening on the
        // client side, so check that a sound server really answers.
        if server_answers(&server) {
            Route::Pulse(server)
        } else {
            Route::Unreachable(server)
        }
    }

    /// Server address to hand to child processes via `PULSE_SERVER`.
    pub fn pulse_server(&self) -> Option<&str> {
        match self {
            #[cfg(feature = "remote")]
            Route::Pulse(s) | Route::Unreachable(s) => Some(s),
            _ => None,
        }
    }
}

#[cfg(feature = "remote")]
/// The newest live forwarded socket. Clients forward to a unique `pulse-ssh-*.sock` per
/// connection (so a leftover from a dropped session can't block the next one), and those
/// leftovers are removed here.
fn forwarded_socket() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    let mut live = Vec::new();
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(SSH_SOCKET_PREFIX) && name.ends_with(".sock")) {
            continue;
        }
        let path = entry.path();
        if UnixStream::connect(&path).is_ok() {
            let modified = entry.metadata().and_then(|m| m.modified()).ok();
            live.push((modified, path));
        } else {
            // Nothing listening: the SSH session that created it is gone.
            let _ = std::fs::remove_file(&path);
        }
    }
    live.into_iter()
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

#[cfg(feature = "remote")]
/// Whether a PulseAudio-protocol handshake succeeds, via `pactl info` with a short timeout.
/// Without `pactl` we can't tell, so assume it's fine.
fn server_answers(server: &str) -> bool {
    let spawned = Command::new("pactl")
        .arg("info")
        .env("PULSE_SERVER", server)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else {
        return true;
    };
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}
