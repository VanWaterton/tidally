#!/data/data/com.termux/files/usr/bin/bash
# EXPERIMENTAL: launcher for tidally-remote from Android (Termux), with sound on the device.
# Read docs/remote-audio.md (especially "Security considerations") before using it.
#
# Usage: ./termux.sh user@host [remote-uid]
#   The target is remembered, so later runs need no arguments. remote-uid is your user id on
#   the host (`id -u` there), default 1000.
set -u
CONF="$HOME/.config/tidally-remote"
mkdir -p "$CONF"
if [ $# -ge 1 ]; then
  echo "$1 ${2:-1000}" > "$CONF/target"
fi
if ! read -r TARGET REMOTE_UID < "$CONF/target" 2>/dev/null; then
  echo "Usage: $0 user@host [remote-uid]"
  exit 2
fi

# 1. Start the device's sound server.
pulseaudio --check 2>/dev/null || pulseaudio --start --exit-idle-time=-1
if ! pactl info >/dev/null 2>&1; then
  echo "✗ The sound server won't start. Run: pulseaudio -k   then try again."
  exit 1
fi

# 2. Let the SSH tunnel reach it (connections from this device only).
pactl list modules short | grep -q module-native-protocol-tcp ||
  pactl load-module module-native-protocol-tcp auth-ip-acl=127.0.0.1 auth-anonymous=1 >/dev/null

# 3. Make sure it plays to the speakers, not a silent "null" output.
sink=$(pactl get-default-sink 2>/dev/null)
if [ -z "$sink" ] || [ "$sink" = auto_null ]; then
  for m in module-aaudio-sink module-sles-sink; do
    pactl load-module "$m" >/dev/null 2>&1 && break
  done
  sink=$(pactl list sinks short | awk '$2 != "auto_null" {print $2; exit}')
  [ -n "$sink" ] && pactl set-default-sink "$sink"
fi
if [ -z "$sink" ] || [ "$sink" = auto_null ]; then
  echo "✗ No speaker output found on this device."
  exit 1
fi
pactl set-sink-mute "$sink" 0 2>/dev/null
echo "✓ Sound ready ($sink)"

termux-wake-lock

# A unique socket per connection, so a leftover from a dropped session can't block the next one.
exec ssh -t -R "/run/user/$REMOTE_UID/pulse-ssh-$(date +%s)-$$.sock:127.0.0.1:4713" \
  "$TARGET" "${TIDALLY_CMD:-tidally-remote}"
