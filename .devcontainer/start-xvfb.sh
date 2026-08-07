#!/usr/bin/env bash
# .devcontainer/start-xvfb.sh
#
# Brings up the headless display the GUI renders onto.
#
# Wired to `postStartCommand` rather than `postCreateCommand` so it runs again after a plain
# container restart, not only after a rebuild -- otherwise the display quietly disappears the
# first time the container is stopped and started.
set -e

# Required, not cosmetic: Vulkan initialisation fails outright with "XDG_RUNTIME_DIR not set in
# the environment", and the directory has to be private to the user to be accepted.
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

# The display server itself. Guarded so a second run is harmless.
if ! pgrep -x Xvfb >/dev/null; then
    Xvfb "$DISPLAY" -screen 0 1280x1024x24 -nolisten tcp &
fi

# Without a window manager X never assigns input focus to the application window. Mouse clicks
# still work, but typed keys are silently discarded while the widget shows a focus ring -- which
# looks exactly like an application bug. openbox assigns focus automatically and costs a few MB.
if ! pgrep -x openbox >/dev/null; then
    # openbox exits immediately if it starts before the display is accepting connections.
    for _ in $(seq 1 50); do
        xdpyinfo -display "$DISPLAY" >/dev/null 2>&1 && break
        sleep 0.1
    done
    openbox &
fi
