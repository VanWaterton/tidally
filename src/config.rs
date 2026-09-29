use std::fmt;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

/// Default device-flow client credentials: a public "Limited Input Device" client used by
/// open-source Tidal tools (e.g. tiddl), which is entitled to lossless (16-bit/44.1kHz FLAC)
/// streams. Override them in `config.toml` if they stop working.
const DEFAULT_CLIENT_ID: &str = "4N3n6Q1x95LL5K7p";
const DEFAULT_CLIENT_SECRET: &str = "oKOXfJW371cX6xaZ0PyhgGNBdNLlBZd4AKKYougMjik=";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Quality {
    Low,
    High,
    #[default]
    Lossless,
    HiResLossless,
}

impl Quality {
    pub fn as_api(self) -> &'static str {
        match self {
            Quality::Low => "LOW",
            Quality::High => "HIGH",
            Quality::Lossless => "LOSSLESS",
            Quality::HiResLossless => "HI_RES_LOSSLESS",
        }
    }
}

impl fmt::Display for Quality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_api())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub quality: Quality,
    pub client_id: String,
    pub client_secret: String,
    pub search_limit: u32,
    /// Equalizer preset name to start with.
    pub eq: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            quality: Quality::default(),
            client_id: DEFAULT_CLIENT_ID.into(),
            client_secret: DEFAULT_CLIENT_SECRET.into(),
            search_limit: 50,
            eq: "Flat".into(),
        }
    }
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        let path = paths.config_file();
        match fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }
}

pub struct Paths {
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub runtime_dir: PathBuf,
}

impl Paths {
    pub fn new() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "tidally").context("no home directory")?;
        let config_dir = dirs.config_dir().to_path_buf();
        let cache_dir = dirs.cache_dir().to_path_buf();
        let runtime_dir = dirs.runtime_dir().unwrap_or(&cache_dir).to_path_buf();
        for dir in [&config_dir, &cache_dir, &runtime_dir] {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(Self {
            config_dir,
            cache_dir,
            runtime_dir,
        })
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn session_file(&self) -> PathBuf {
        self.config_dir.join("session.json")
    }

    pub fn mpv_socket(&self) -> PathBuf {
        self.runtime_dir
            .join(format!("mpv-{}.sock", std::process::id()))
    }
}
