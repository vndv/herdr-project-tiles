#!/bin/sh
# Launcher: the herdr server may run with a minimal PATH (launchd), so look for python3 explicitly.
here=$(cd "$(dirname "$0")/.." && pwd)
for py in /opt/homebrew/bin/python3 /usr/local/bin/python3 "$(command -v python3 2>/dev/null)" /usr/bin/python3; do
  if [ -n "$py" ] && [ -x "$py" ]; then
    exec "$py" -I "$here/tiles.py" "$@"
  fi
done
echo "project-tiles: python3 (>= 3.11) not found" >&2
sleep 3
exit 1
