# Tidally

A slick terminal client for [Tidal](https://tidal.com), written in Rust with
[ratatui](https://ratatui.rs). Playback goes through `mpv`.

- **Keyword search:** `artist radiohead`, `album ok computer`, `track creep artist radiohead`
- **Album art** in the terminal (kitty, sixel or iTerm2 graphics, with a coloured-block fallback)
- **Colours taken from the album cover** for the whole interface
- **Spectrum visualizer** and a **10-band EQ** with presets
- **Queue** with reordering, a **Now Playing** view, and **touch/mouse** support (works well in
  Termux on a tablet)

> **Unofficial.** tidally uses Tidal's private API through the same device-login flow as
> other open-source clients. It isn't affiliated with or endorsed by Tidal, needs a paid
> subscription, and may break if Tidal changes their API. Use it for personal listening, in
> line with Tidal's terms.

<img width="1920" height="1080" alt="Image" src="https://github.com/user-attachments/assets/68b44ed4-f878-486d-a475-3dfe1e818ddb" />

## Requirements

- Linux (other Unix systems may work but aren't tested)
- [`mpv`](https://mpv.io) on your `PATH`
- Optional: `parec` (from `libpulse` / `pulseaudio-utils`, which PipeWire systems usually
  have) for the spectrum visualizer
- A terminal with true colour. For real album art, use kitty, WezTerm, Konsole, foot, Ghostty
  or iTerm2; others get block-art.

## Install

**Prebuilt binary (Linux x86_64 / aarch64):** download the latest archive from
[Releases](https://github.com/VanWaterton/tidally/releases), extract it, and put
`tidally` somewhere on your `PATH`.

**With Cargo:**

1. Install the dependencies:

   | Distro | Command |
   |---|---|
   | Arch / Manjaro / CachyOS | `sudo pacman -S --needed base-devel mpv libpulse` |
   | Debian / Ubuntu / Mint | `sudo apt install build-essential mpv pulseaudio-utils` |
   | Fedora | `sudo dnf install gcc mpv pulseaudio-utils` (mpv comes from [RPM Fusion](https://rpmfusion.org/Configuration)) |

2. Install Rust 1.90 or newer from [rustup.rs](https://rustup.rs) (distro packages are often
   older).
3. Build and install Tidally:

   ```sh
   cargo install --git https://github.com/VanWaterton/tidally --locked
   ```

   This puts `tidally` in `~/.cargo/bin`. If your shell can't find it, open a new terminal.

**Arch Linux:** a `-git` PKGBUILD is in [`packaging/arch`](packaging/arch/PKGBUILD).

## Usage

```sh
tidally
```

On first launch, open the link shown (press `o`) and approve the device in your browser. The
session is stored in `~/.config/tidally/session.json` (readable only by you).
`tidally --logout` forgets it.

### Search

Type plain text, or combine `artist`, `album` and `track` in any order:

| Query | Result |
|---|---|
| `paranoid android` | plain track search |
| `artist radiohead` | the artist's top tracks |
| `album ok computer` | the whole album, in order |
| `artist radiohead album kid a` | that artist's album, so a same-named album by someone else doesn't match |
| `track creep artist radiohead` | tracks filtered by every field you gave |

`artist:name` works too, and quotes keep a keyword from being read as one: `album "the artist"`.

### Keys

| Key | Action |
|---|---|
| `/` | Search box (Enter runs the search; `↑` from the first result also returns here) |
| `Tab` / `1` `2` `3` | Search / Queue / Now Playing |
| `j` `k` / `↑` `↓`, `g` `G` | Move |
| `Enter` | Play. In search, this replaces the queue with the results, starting at the selection |
| `a` / `N` | Add to queue / play next |
| `d`, `J` `K`, `c` | Queue: remove, reorder, clear |
| `Space` | Pause |
| `n` / `p` | Next / previous (previous restarts the track if you're more than 3s in) |
| `←` `→` / `h` `l` | Seek ±5s |
| `+` / `-` | Volume |
| `e` / `E` | Next / previous EQ preset |
| `?` | Help |
| `q` / `Ctrl-C` | Quit |

### Touch and mouse

- Tap the search box to type in it.
- Tap a song to select it, and tap it again to play.
- Swipe (or scroll) to move through lists.
- Tap the tabs, the ⏮ ▶ ⏭ buttons, the progress bar (to seek) or the EQ label (next preset).

Because the app captures the mouse, selecting text in a desktop terminal needs **Shift+drag**.

## Configuration

`~/.config/tidally/config.toml`, where every key is optional:

```toml
quality = "LOSSLESS"      # LOW | HIGH | LOSSLESS | HI_RES_LOSSLESS
search_limit = 50
eq = "Flat"               # starting EQ preset: Flat, Bass Boost, Loudness, Rock, Electronic,
                          # Hip-Hop, Vocal, Acoustic, Treble Boost
# client_id = "..."       # override if the built-in device client stops working
# client_secret = "..."
```

`TIDALLY_NO_GRAPHICS=1` forces block-art if your terminal's image support misbehaves.

## Notes

- **Stream quality:** the built-in login client streams up to **lossless FLAC (16-bit /
  44.1 kHz)**. Hi-res isn't available through it. The player bar shows what's actually being
  streamed.
- **Visualizer:** it reads the default output's monitor with `parec`, like cava, so it shows
  all system audio, not only tidally.
- **EQ:** presets are applied as an ffmpeg filter inside mpv. Each preset lowers the overall
  level by its largest boost to avoid clipping.
  

## Roadmap

- Hi-res (24-bit) streams
- Albums, artists, playlists and favourites views
- Gapless playback
- MPRIS (media keys, `playerctl`)
- Synced lyrics
- Remembering the queue and volume between runs
- Remote stream

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Bug reports and pull requests are welcome.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT),
at your option.
