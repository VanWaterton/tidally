# tidally-remote (experimental)

> **Status: experimental, not released.** It isn't included in release downloads or in a
> default `cargo install`, and the security considerations below are still under review.

`tidally-remote` is the same app as `tidally`, with one addition. When you run it on a
machine over SSH, it can play through **your client device's** speakers instead of the host's.
The client forwards its PulseAudio / PipeWire-pulse socket over the SSH connection, and
`tidally-remote` detects it at startup.

## Build

```sh
cargo install --git https://github.com/VanWaterton/tidally --locked --features remote
```

This installs both `tidally` and `tidally-remote`.

## How it works

1. The client runs a sound server (PipeWire-pulse, PulseAudio, or PulseAudio in Termux on
   Android).
2. The client connects with `ssh -R /run/user/<uid>/pulse-ssh-<unique>.sock:<local socket or 127.0.0.1:4713>`.
   Each connection uses a unique name, so a socket left behind by a dropped session never
   blocks the next one.
3. On the host, `tidally-remote`:
   - picks the newest `pulse-ssh*.sock` in `$XDG_RUNTIME_DIR` that has a listener, and deletes
     dead ones;
   - confirms that a sound server actually answers (`pactl info`), because SSH accepts
     connections on a forwarded socket even when nothing is listening on the client side;
   - starts mpv with `--ao=pulse` and `PULSE_SERVER` pointing at the socket (mpv's default
     PipeWire output would ignore it), and points the visualizer's `parec` at it as well.
4. The header shows `⇄ remote audio` when it works, `✗ remote audio` (red) when the forwarded
   server doesn't answer, and `host speakers` (red) when you're over SSH with no forward.
   Setting `PULSE_SERVER` yourself also works.

Audio is decoded on the host and sent as uncompressed PCM (about 1.4 Mbit/s). That's fine on
a LAN, but heavy on slow links.

## Clients

- **Linux:** `contrib/remote/linux.sh user@host [remote-uid]`
- **Android (Termux):** install Termux from F-Droid, `pkg install openssh pulseaudio`, copy
  `contrib/remote/termux.sh` to the device, then run `./termux.sh user@host [remote-uid]` once.
  After that, `./termux.sh` is enough. Set Termux's battery usage to *Unrestricted* so
  playback continues with the screen off.

`remote-uid` is your numeric user id on the host (`id -u`), default 1000.

## Security considerations (under review)

These are the known issues to settle before a release:

1. **The forwarded socket is a full PulseAudio connection to the client's sound server.**
   Anything running as your user on the host can connect to it while the session is up. It
   can do more than play audio: through the native protocol it can list and *record from*
   the client's sources (for example a microphone, if the client's server exposes one), and
   change volumes. The host effectively gets control of the client's audio system for as
   long as the tunnel is up.
   - *Possible fix:* stream in one direction only. mpv on the host writes PCM to stdout, it
     travels over the SSH channel, and the client plays it with `pacat --playback`. The
     client then exposes nothing.
2. **The Termux script loads `module-native-protocol-tcp` with `auth-anonymous=1`** on
   `127.0.0.1:4713`. Any other app on the Android device can use that server without
   authentication, including recording if a source exists, until PulseAudio stops.
   - *Possible fix:* cookie authentication, a Unix socket instead of TCP, or fix 1 above
     (which removes the need for this).
3. **Host-side socket permissions.** sshd creates the forwarded socket in `/run/user/<uid>`
   (a `0700` directory) with `StreamLocalBindMask` (default `0177`, giving mode `0600`), so
   other users on the host can't reach it. This relies on sshd defaults, so check them if
   your sshd config differs.
4. **Cleanup deletes `pulse-ssh*.sock` files** in `$XDG_RUNTIME_DIR` that have no listener.
   The directory is private to your user, but the prefix is generic, so another tool using
   the same prefix could have its dead sockets removed.
5. **`pactl` is run from `PATH`** for the health check. Worth considering whether that's
   acceptable, or whether the check should be done in-process.
6. **`termux-wake-lock`** keeps the device awake until released (`termux-wake-unlock`). That's
   a battery issue, not a security one, but it isn't released automatically.
