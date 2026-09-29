# Security

## Reporting a vulnerability

Please report security issues privately through GitHub's
[private vulnerability reporting](https://github.com/VanWaterton/tidally/security/advisories/new)
rather than in a public issue.

## What tidally stores

- `~/.config/tidally/session.json`: your Tidal access and refresh tokens, created with
  mode `0600`. Delete it (or run `tidally --logout`) to sign out.
- `~/.cache/tidally/`: temporary stream manifests.

The built-in Tidal client credentials are the public ones used by other open-source Tidal
clients. They identify the app, not you.

## Remote edition

The experimental `tidally-remote` edition has known open security questions and isn't
released. See [docs/remote-audio.md](docs/remote-audio.md#security-considerations-under-review).
