#!/usr/bin/env bash
# EXPERIMENTAL: run tidally-remote on another machine, with sound on this Linux desktop
# (PipeWire or PulseAudio). Read docs/remote-audio.md before using it.
#
# Usage: ./linux.sh user@host [remote-uid]
set -eu
TARGET=${1:?usage: $0 user@host [remote-uid]}
REMOTE_UID=${2:-1000}
LOCAL_SOCK="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/pulse/native"
if [ ! -S "$LOCAL_SOCK" ]; then
  echo "No PulseAudio / pipewire-pulse socket at $LOCAL_SOCK" >&2
  exit 1
fi
# A unique socket per connection, so a leftover from a dropped session can't block the next one.
exec ssh -t -R "/run/user/$REMOTE_UID/pulse-ssh-$(date +%s)-$$.sock:$LOCAL_SOCK" \
  "$TARGET" "${TIDALLY_CMD:-tidally-remote}"
