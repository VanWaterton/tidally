# Contributing

Thanks for helping out! Bug reports, ideas and pull requests are all welcome.

## Development

```sh
cargo run                          # the local edition (tidally)
cargo run --features remote --bin tidally-remote   # experimental remote edition
```

You need `mpv` on your `PATH`, and a Tidal subscription to test playback.

## Before opening a PR

CI runs these, so run them first:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features remote -- -D warnings
cargo test --all-features
```

## Layout

```
src/
  lib.rs         startup, terminal setup, `Edition` (local / remote)
  bin/           the two thin executables
  app.rs         state, key and touch handling, async action dispatch
  event.rs       messages from background tasks to the app
  ui/            rendering and theme
  tidal/         API client, device-code auth, models
  player.rs      mpv process over JSON IPC
  search.rs      query syntax, plus album/artist/track resolution
  art.rs         cover download, palette extraction, off-thread image encoding
  visualizer.rs  parec capture, FFT, log-spaced bands
  eq.rs          EQ presets as an mpv lavfi filter chain
  audio.rs       output routing (remote edition: SSH-forwarded sound server)
  config.rs      config file and XDG paths
```

Code that only the remote edition needs is behind `#[cfg(feature = "remote")]`, so the local
build contains none of it. Please keep it that way.
