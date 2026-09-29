//! Drives a headless mpv process over its JSON IPC socket.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::audio::Route;
use crate::event::AppEvent;

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Position(f64),
    Duration(f64),
    Paused(bool),
    Volume(f64),
    /// The current file finished playing on its own (not replaced or stopped).
    Finished,
    Failed(String),
    Exited,
}

const OBSERVED: [&str; 4] = ["time-pos", "duration", "pause", "volume"];

pub struct Player {
    tx: UnboundedSender<Value>,
    socket: PathBuf,
    _child: Child,
}

impl Player {
    pub async fn spawn(
        socket: &Path,
        route: &Route,
        events: UnboundedSender<AppEvent>,
    ) -> Result<Self> {
        let _ = std::fs::remove_file(socket);
        let mut cmd = Command::new("mpv");
        #[cfg(feature = "remote")]
        if let Some(server) = route.pulse_server() {
            // mpv prefers its native PipeWire output, which ignores PULSE_SERVER.
            cmd.arg("--ao=pulse").env("PULSE_SERVER", server);
        }
        #[cfg(not(feature = "remote"))]
        let _ = route;
        let child = cmd
            .arg("--idle=yes")
            .arg("--no-video")
            .arg("--no-terminal")
            .arg("--ytdl=no")
            .arg("--audio-display=no")
            .arg("--gapless-audio=weak")
            // DASH manifests are written to a local file but reference remote segments.
            .arg("--demuxer-lavf-o=protocol_whitelist=[file,http,https,tcp,tls,crypto,data]")
            .arg(format!("--input-ipc-server={}", socket.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("could not start mpv (is it installed and on PATH?)")?;

        let stream = connect(socket).await?;
        let (read, mut write) = stream.into_split();

        let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let mut line = msg.to_string();
                line.push('\n');
                if write.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        });

        tokio::spawn(async move {
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(ev) = serde_json::from_str::<Value>(&line)
                    .ok()
                    .and_then(|v| parse_event(&v))
                    && events.send(AppEvent::Player(ev)).is_err()
                {
                    return;
                }
            }
            let _ = events.send(AppEvent::Player(PlayerEvent::Exited));
        });

        let player = Self {
            tx,
            socket: socket.to_path_buf(),
            _child: child,
        };
        for (id, name) in OBSERVED.iter().enumerate() {
            player.command(json!(["observe_property", id + 1, name]));
        }
        Ok(player)
    }

    fn command(&self, args: Value) {
        let _ = self.tx.send(json!({ "command": args }));
    }

    pub fn load(&self, target: &str) {
        self.command(json!(["loadfile", target, "replace"]));
        self.command(json!(["set_property", "pause", false]));
    }

    pub fn stop(&self) {
        self.command(json!(["stop"]));
    }

    pub fn toggle_pause(&self) {
        self.command(json!(["cycle", "pause"]));
    }

    pub fn seek_relative(&self, secs: f64) {
        self.command(json!(["seek", secs, "relative"]));
    }

    pub fn seek_absolute(&self, secs: f64) {
        self.command(json!(["seek", secs, "absolute"]));
    }

    /// Replaces mpv's audio filter chain; `None` clears it.
    pub fn set_audio_filter(&self, filter: Option<&str>) {
        match filter {
            Some(f) => self.command(json!(["af", "set", f])),
            None => self.command(json!(["af", "clr", ""])),
        }
    }

    pub fn add_volume(&self, delta: f64) {
        self.command(json!(["add", "volume", delta]));
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

async fn connect(socket: &Path) -> Result<UnixStream> {
    let mut last_err = None;
    for _ in 0..100 {
        match UnixStream::connect(socket).await {
            Ok(s) => return Ok(s),
            Err(e) => last_err = Some(e),
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    Err(last_err.unwrap()).context("timed out connecting to mpv IPC socket")
}

fn parse_event(v: &Value) -> Option<PlayerEvent> {
    match v.get("event")?.as_str()? {
        "property-change" => {
            let data = v.get("data");
            match v.get("name")?.as_str()? {
                "time-pos" => Some(PlayerEvent::Position(data?.as_f64()?)),
                "duration" => Some(PlayerEvent::Duration(data?.as_f64()?)),
                "pause" => Some(PlayerEvent::Paused(data?.as_bool()?)),
                "volume" => Some(PlayerEvent::Volume(data?.as_f64()?)),
                _ => None,
            }
        }
        "end-file" => match v.get("reason")?.as_str()? {
            "eof" => Some(PlayerEvent::Finished),
            "error" => Some(PlayerEvent::Failed(
                v.get("file_error")
                    .and_then(Value::as_str)
                    .unwrap_or("playback error")
                    .to_string(),
            )),
            _ => None,
        },
        _ => None,
    }
}
