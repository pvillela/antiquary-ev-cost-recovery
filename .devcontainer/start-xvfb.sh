#!/usr/bin/env bash
# .devcontainer/start-xvfb.sh
#
# Brings up the headless display the GUI renders onto.
#
# Wired to `postStartCommand` rather than `postCreateCommand` so it runs again after a plain
# container restart, not only after a rebuild -- otherwise the display quietly disappears the
# first time the container is stopped and started.
set -e

# Both daemons log here rather than to the caller's stdout. A backgrounded child inherits
# postStartCommand's pipe, and holding that pipe open makes the devcontainer's startup step appear
# to hang long after this script has exited.
LOG=/tmp/xvfb-startup.log

# Required, not cosmetic: Vulkan initialisation fails outright with "XDG_RUNTIME_DIR not set in
# the environment", and the directory has to be private to the user to be accepted.
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

# The Dockerfile creates this as root with mode 1777, which is what silences the
# "_XSERVTransmkdir: euid != 0" warning at source. This line is only the fallback for a container
# built from an image predating that: a user-owned directory is enough for Xvfb to bind in.
mkdir -p /tmp/.X11-unix

# The display server itself. Guarded so a second run is harmless. `setsid` puts it in its own
# process group so it is not signalled when postStartCommand finishes.
if ! pgrep -x Xvfb >/dev/null; then
    setsid Xvfb "$DISPLAY" -screen 0 1280x1024x24 -nolisten tcp >>"$LOG" 2>&1 &
fi

# Wait for the server to accept connections before starting anything that talks to it.
for _ in $(seq 1 100); do
    if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then break; fi
    sleep 0.1
done
if ! xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
    echo "start-xvfb.sh: display $DISPLAY did not come up; see $LOG" >&2
    exit 1
fi

# Without a window manager X never assigns input focus to the application window. Mouse clicks
# still work, but typed keys are silently discarded while the widget shows a focus ring -- which
# looks exactly like an application bug. openbox assigns focus automatically and costs a few MB.
#
# openbox exits immediately if it starts before the display is accepting connections. The earlier
# version launched it as soon as the wait loop ended, whether or not the loop had succeeded, and
# never checked that it stayed up -- so a slow Xvfb left a working display with no window manager
# and nothing said so. That is why the wait above is fatal and why this checks it survived.
for _ in 1 2; do
    if pgrep -x openbox >/dev/null; then break; fi
    setsid openbox >>"$LOG" 2>&1 &
    sleep 1
done
if ! pgrep -x openbox >/dev/null; then
    echo "start-xvfb.sh: openbox did not start; typed keys will be ignored. See $LOG" >&2
fi
