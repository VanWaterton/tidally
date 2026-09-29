use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: u64,
    pub title: String,
    pub version: Option<String>,
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub artists: Vec<ArtistRef>,
    pub album: Option<AlbumRef>,
    #[serde(default)]
    pub explicit: bool,
}

impl Track {
    pub fn display_title(&self) -> String {
        match self.version.as_deref().filter(|v| !v.is_empty()) {
            Some(v) => format!("{} ({v})", self.title),
            None => self.title.clone(),
        }
    }

    pub fn artist_names(&self) -> String {
        self.artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn album_title(&self) -> &str {
        self.album.as_ref().map_or("", |a| a.title.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtistRef {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlbumRef {
    pub title: String,
    pub cover: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Page<T> {
    #[serde(default = "Vec::new")]
    pub items: Vec<T>,
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<ArtistRef>,
}

impl Album {
    pub fn artist_names(&self) -> String {
        self.artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artist {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchResults {
    #[serde(default)]
    pub tracks: Page<Track>,
    #[serde(default)]
    pub albums: Page<Album>,
    #[serde(default)]
    pub artists: Page<Artist>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlaybackInfo {
    pub audio_quality: Option<String>,
    pub manifest_mime_type: String,
    pub manifest: String,
    pub bit_depth: Option<u32>,
    pub sample_rate: Option<u32>,
}

/// Manifest format `application/vnd.tidal.bts`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BtsManifest {
    pub codecs: Option<String>,
    pub encryption_type: Option<String>,
    #[serde(default)]
    pub urls: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum StreamSource {
    /// A direct URL mpv can open.
    Url(String),
    /// A DASH MPD document that has to be written to disk for mpv.
    Dash(String),
}

#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub source: StreamSource,
    pub quality: Option<String>,
    pub codec: Option<String>,
    pub bit_depth: Option<u32>,
    pub sample_rate: Option<u32>,
}

impl StreamInfo {
    /// Short badge like `FLAC 24/96` or `LOSSLESS`.
    pub fn badge(&self) -> String {
        let codec = self.codec.as_deref().map(|c| {
            let base = c.split('.').next().unwrap_or(c).to_lowercase();
            match base.as_str() {
                "mp4a" => "AAC".to_string(),
                other => other.to_uppercase(),
            }
        });
        match (codec, self.bit_depth, self.sample_rate) {
            (Some(c), Some(b), Some(r)) => format!("{c} {b}/{}", format_rate(r)),
            (Some(c), _, _) if c == "AAC" => match self.quality.as_deref() {
                Some("HIGH") => "AAC 320".into(),
                Some("LOW") => "AAC 96".into(),
                _ => c,
            },
            (Some(c), _, _) => c,
            _ => self.quality.clone().unwrap_or_default(),
        }
    }
}

fn format_rate(hz: u32) -> String {
    if hz.is_multiple_of(1000) {
        (hz / 1000).to_string()
    } else {
        format!("{:.1}", hz as f64 / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(codec: &str, quality: &str, depth: Option<u32>, rate: Option<u32>) -> StreamInfo {
        StreamInfo {
            source: StreamSource::Url(String::new()),
            quality: Some(quality.into()),
            codec: Some(codec.into()),
            bit_depth: depth,
            sample_rate: rate,
        }
    }

    #[test]
    fn badges() {
        assert_eq!(info("mp4a.40.2", "HIGH", None, None).badge(), "AAC 320");
        assert_eq!(
            info("flac", "LOSSLESS", Some(16), Some(44100)).badge(),
            "FLAC 16/44.1"
        );
        assert_eq!(
            info("flac", "HI_RES_LOSSLESS", Some(24), Some(96000)).badge(),
            "FLAC 24/96"
        );
    }
}
