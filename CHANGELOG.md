# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-09-29

First release of `tidally` (the local edition).

### Added
- Device-code login with automatic token refresh; the session is stored with mode 0600.
- Lossless FLAC (16-bit / 44.1 kHz) streaming. Streams split into several files play as one
  seamless track.
- Search with `artist` / `album` / `track` keywords, quoting and `key:value` forms.
- Playback through mpv: queue, next/previous, seek, volume, play-next.
- Album art through kitty, sixel or iTerm2 graphics, with a coloured-block fallback.
- Interface colours taken from the current album cover.
- Spectrum visualizer (`parec` monitor capture and FFT).
- 10-band EQ presets applied as an mpv audio filter.
- Now Playing view, touch and mouse support, and a help overlay.
- Experimental `tidally-remote` edition behind the `remote` Cargo feature (not released).
