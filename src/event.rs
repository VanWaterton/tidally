use std::sync::Arc;

use image::DynamicImage;
use ratatui::layout::Size;
use ratatui_image::protocol::Protocol;

use crate::art::Palette;
use crate::player::PlayerEvent;
use crate::search::Outcome;
use crate::tidal::auth::{DeviceCode, Session};
use crate::tidal::models::StreamInfo;

/// Everything that can happen asynchronously and needs the app's attention.
pub enum AppEvent {
    LoginCode(DeviceCode),
    LoggedIn(Session),
    LoginFailed(String),
    SearchResults {
        query: String,
        result: Result<Outcome, String>,
    },
    StreamReady {
        generation: u64,
        result: Result<StreamInfo, String>,
    },
    Player(PlayerEvent),
    CoverLoaded {
        cover: String,
        image: Arc<DynamicImage>,
        palette: Palette,
    },
    CoverEncoded {
        cover: String,
        size: Size,
        protocol: Protocol,
    },
}
