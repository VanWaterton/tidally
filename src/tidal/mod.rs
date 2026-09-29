pub mod auth;
pub mod models;

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use serde::de::DeserializeOwned;
use tokio::sync::RwLock;

use crate::config::Quality;
use auth::{Credentials, Session};
use models::{
    Album, Artist, BtsManifest, Page, PlaybackInfo, SearchResults, StreamInfo, StreamSource, Track,
};

pub const API_BASE: &str = "https://api.tidal.com/v1";

pub struct Client {
    http: reqwest::Client,
    creds: Credentials,
    session: RwLock<Session>,
    session_path: PathBuf,
    quality: Quality,
}

impl Client {
    pub fn new(
        http: reqwest::Client,
        creds: Credentials,
        session: Session,
        session_path: PathBuf,
        quality: Quality,
    ) -> Self {
        Self {
            http,
            creds,
            session: RwLock::new(session),
            session_path,
            quality,
        }
    }

    pub async fn search_tracks(&self, query: &str, limit: u32) -> Result<Vec<Track>> {
        Ok(self.search(query, "TRACKS", limit).await?.tracks.items)
    }

    pub async fn search_albums(&self, query: &str, limit: u32) -> Result<Vec<Album>> {
        Ok(self.search(query, "ALBUMS", limit).await?.albums.items)
    }

    pub async fn search_artists(&self, query: &str, limit: u32) -> Result<Vec<Artist>> {
        Ok(self.search(query, "ARTISTS", limit).await?.artists.items)
    }

    async fn search(&self, query: &str, types: &str, limit: u32) -> Result<SearchResults> {
        self.get(
            "/search",
            &[
                ("query", query.to_string()),
                ("types", types.to_string()),
                ("limit", limit.to_string()),
            ],
        )
        .await
    }

    pub async fn album_tracks(&self, album_id: u64) -> Result<Vec<Track>> {
        let page: Page<Track> = self
            .get(
                &format!("/albums/{album_id}/tracks"),
                &[("limit", "100".into())],
            )
            .await?;
        Ok(page.items)
    }

    pub async fn artist_top_tracks(&self, artist_id: u64, limit: u32) -> Result<Vec<Track>> {
        let page: Page<Track> = self
            .get(
                &format!("/artists/{artist_id}/toptracks"),
                &[("limit", limit.to_string())],
            )
            .await?;
        Ok(page.items)
    }

    pub async fn stream(&self, track_id: u64) -> Result<StreamInfo> {
        let info: PlaybackInfo = self
            .get(
                &format!("/tracks/{track_id}/playbackinfopostpaywall"),
                &[
                    ("audioquality", self.quality.as_api().into()),
                    ("playbackmode", "STREAM".into()),
                    ("assetpresentation", "FULL".into()),
                ],
            )
            .await?;

        let raw = base64::engine::general_purpose::STANDARD
            .decode(info.manifest.trim())
            .context("manifest is not valid base64")?;

        let (source, codec) = match info.manifest_mime_type.as_str() {
            "application/vnd.tidal.bts" => {
                let bts: BtsManifest = serde_json::from_slice(&raw).context("bad BTS manifest")?;
                if bts.encryption_type.as_deref().is_some_and(|e| e != "NONE") {
                    bail!("track stream is encrypted, which isn't supported");
                }
                let url =
                    join_segments(&bts.urls).ok_or_else(|| anyhow!("manifest had no URLs"))?;
                (StreamSource::Url(url), bts.codecs)
            }
            "application/dash+xml" => {
                let mpd = String::from_utf8(raw).context("DASH manifest is not UTF-8")?;
                let codec = mpd
                    .split("codecs=\"")
                    .nth(1)
                    .and_then(|s| s.split('"').next())
                    .map(str::to_string);
                (StreamSource::Dash(mpd), codec)
            }
            other => bail!("unsupported manifest type {other}"),
        };

        Ok(StreamInfo {
            source,
            quality: info.audio_quality,
            codec,
            bit_depth: info.bit_depth,
            sample_rate: info.sample_rate,
        })
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        let stale = {
            let s = self.session.read().await;
            s.tokens
                .expires_soon()
                .then(|| s.tokens.access_token.clone())
        };
        if let Some(token) = stale {
            self.refresh(&token).await?;
        }
        for attempt in 0..2 {
            let (token, country) = {
                let s = self.session.read().await;
                (s.tokens.access_token.clone(), s.country_code.clone())
            };
            let resp = self
                .http
                .get(format!("{API_BASE}{path}"))
                .bearer_auth(&token)
                .query(&[("countryCode", country)])
                .query(query)
                .send()
                .await?;
            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                self.refresh(&token).await?;
                continue;
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                let msg = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| {
                        v.get("userMessage")
                            .and_then(|m| m.as_str())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| body.chars().take(120).collect());
                bail!("HTTP {status}: {msg}");
            }
            return resp
                .json()
                .await
                .with_context(|| format!("decoding {path}"));
        }
        bail!("unauthorized after token refresh")
    }

    /// Refreshes the access token, unless another request already replaced `used_token`.
    async fn refresh(&self, used_token: &str) -> Result<()> {
        let mut session = self.session.write().await;
        if session.tokens.access_token != used_token {
            return Ok(());
        }
        session.tokens = auth::refresh(&self.http, &self.creds, &session.tokens).await?;
        session.save(&self.session_path)
    }
}

/// A single mpv-playable URL for a stream that may be split into several files. Multiple
/// segments become an mpv EDL (`edl://`), which mpv plays as one seamless file, so the player
/// sees one track (one duration, one end-of-file).
fn join_segments(urls: &[String]) -> Option<String> {
    match urls {
        [] => None,
        [one] => Some(one.clone()),
        many => {
            // `%<byte length>%` quotes each URL, so `;` or `,` inside one can't break the list.
            let parts: Vec<String> = many.iter().map(|u| format!("%{}%{u}", u.len())).collect();
            Some(format!("edl://{}", parts.join(";")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::join_segments;

    #[test]
    fn segments() {
        assert_eq!(join_segments(&[]), None);
        assert_eq!(
            join_segments(&["https://a/x.flac".into()]).as_deref(),
            Some("https://a/x.flac")
        );
        assert_eq!(
            join_segments(&["https://a/1?x=1;2".into(), "https://a/2".into()]).as_deref(),
            Some("edl://%17%https://a/1?x=1;2;%11%https://a/2")
        );
    }
}
