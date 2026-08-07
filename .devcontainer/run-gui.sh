#!/usr/bin/env bash
# .devcontainer/run-gui.sh
#
# Launches the GUI on the headless display, wrapped in a DBus session.
#
# The wrapper is what makes the file dialogs work. `ev_peak_gui` opens files through `rfd`
# (src/bin/ev_peak_gui/widgets.rs), and rfd 0.17 defaults to its xdg-portal backend; with no
# portal reachable the dialog fails *silently* -- clicking the button does nothing at all, with
# no error and no panic. Running through this script rather than a bare `cargo run` is what keeps
# that from being rediscovered as an application bug.
#
# Usage:
#   bash .devcontainer/run-gui.sh                    # cargo run --bin ev_peak_gui
#   bash .devcontainer/run-gui.sh --release          # any cargo run arguments
set -e

if [ $# -eq 0 ]; then
    set -- --bin ev_peak_gui
fi

exec dbus-run-session -- cargo run "$@"
