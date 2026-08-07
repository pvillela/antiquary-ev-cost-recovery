# Running GUI (eframe/egui) Apps in the Devcontainer

Recommendations for enabling GUI examples such as `examples/google_eframe.rs` to run inside
the devcontainer, including headless execution driven by an agent.

Status: **verified working**. Every package and setting below was installed and exercised in a
live container on 2026-07-26 — the example was launched headlessly, the date-picker popup was
opened and cancelled via synthetic clicks, and the `rfd` file chooser was opened with the
`.xlsx/.xls/.xlsm` filter applied. Screenshots confirmed correct rendering.

**Applied on 2026-08-07:** Option A (headless Xvfb), with the Part 1 packages moved into
`.devcontainer/Dockerfile` so they survive a rebuild as a cached layer rather than being
reinstalled by `setup.sh` each time, and `openbox` added to settle the focus gotcha in Part 3.
Options B and C were not taken. See `.devcontainer/start-xvfb.sh` and `.devcontainer/run-gui.sh`.

---

## Background: why it does not work today

The container has none of the GUI runtime pieces:

- No X or Wayland display (`DISPLAY` and `WAYLAND_DISPLAY` both unset), no `/tmp/.X11-unix`.
- No X client libraries. The example still *links* because `winit` loads `libX11`/`libwayland`
  lazily via `dlopen`, so the failure only appears at runtime.
- No GPU (`/dev/dri` absent) and no software rasterizer.
- No DBus session and no desktop portal, which `rfd` needs for file dialogs.

One point specific to the 0.29 -> 0.35 upgrade: **eframe 0.35 defaults to the `wgpu` renderer**
(0.29 defaulted to `glow`). So a GPU driver is now required. With no `/dev/dri`, that means a
Mesa software driver — lavapipe (Vulkan) or llvmpipe (GL). Both were tested and both work:

```
WGPU_BACKEND=vulkan   -> window created (lavapipe, Vulkan 1.3)
WGPU_BACKEND=gl       -> window created (llvmpipe)
```

Installing both driver sets gives a fallback if one misbehaves.

---

## Part 1 — Required for all options: packages

Add to `.devcontainer/setup.sh` (runs via `postCreateCommand`):

```bash
echo "Installing GUI runtime ..."
sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
  xvfb x11-utils xdotool imagemagick \
  libx11-6 libxcb1 libxkbcommon-x11-0 libxcursor1 libxrandr2 libxi6 libwayland-client0 \
  libgl1 libegl1 libgl1-mesa-dri libvulkan1 mesa-vulkan-drivers \
  fonts-dejavu-core \
  dbus-x11 xdg-desktop-portal xdg-desktop-portal-gtk
```

What each group is for:

| Group | Purpose |
|---|---|
| `xvfb` | Virtual X display — the display server itself. |
| `x11-utils`, `xdotool`, `imagemagick` | Inspect windows (`xwininfo`), synthesise clicks/keys (`xdotool`), capture screenshots (`import`). These are what make an agent able to *drive and observe* the app, not merely launch it. |
| `libx11-6`, `libxcb1`, `libxkbcommon-x11-0`, `libxcursor1`, `libxrandr2`, `libxi6`, `libwayland-client0` | X/Wayland client libraries `winit` dlopens at runtime. |
| `libgl1`, `libegl1`, `libgl1-mesa-dri`, `libvulkan1`, `mesa-vulkan-drivers` | Software rendering for wgpu: llvmpipe (GL) and lavapipe (Vulkan). |
| `fonts-dejavu-core` | egui bundles its own fonts, but the GTK file dialog needs system fonts. |
| `dbus-x11`, `xdg-desktop-portal`, `xdg-desktop-portal-gtk` | `rfd` file dialogs — see Part 4. |

Optional extras: `vulkan-tools` (`vulkaninfo --summary`) and `mesa-utils` (`glxinfo -B`) are
useful for diagnosing driver problems but are not needed to run anything.

> Note: this adds a few hundred MB and re-runs on every container rebuild. If that becomes
> tedious, move the block into a `Dockerfile` with `"build": {"dockerfile": "Dockerfile"}` in
> `devcontainer.json` so it becomes a cached image layer.

---

## Part 2 — Choose a display strategy

Three mutually exclusive options for *where* the GUI is displayed. `DISPLAY` can only point at
one of them, so pick one as the default.

### Option A — Headless Xvfb (recommended for agent-driven verification)

No visible window; the agent runs the app and reads screenshots. Works regardless of the host
OS, over SSH, and in CI.

`.devcontainer/devcontainer.json`:

```json
  "containerEnv": {
    "DISPLAY": ":99",
    "XDG_RUNTIME_DIR": "/tmp/xdg-runtime",
    "XDG_CURRENT_DESKTOP": "GNOME"
  },
  "postStartCommand": "bash .devcontainer/start-xvfb.sh",
```

New `.devcontainer/start-xvfb.sh`:

```bash
#!/usr/bin/env bash
set -e
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
pgrep -x Xvfb >/dev/null || Xvfb :99 -screen 0 1280x1024x24 -nolisten tcp &
```

Use `postStartCommand`, not `postCreateCommand`, so Xvfb comes back after a container restart.

`XDG_RUNTIME_DIR` is genuinely required, not cosmetic: Vulkan initialisation fails with
`XDG_RUNTIME_DIR not set in the environment` without it. `XDG_CURRENT_DESKTOP` tells the
desktop portal which backend to use.

Pros: simple, host-independent, no security surface.
Cons: you see nothing directly — only screenshots the agent captures.

### Option B — noVNC desktop in a browser

Adds a lightweight window manager plus a browser-accessible desktop, so both you and the agent
share one display.

```json
  "features": {
    "ghcr.io/devcontainers/features/desktop-lite:1": {}
  }
```

Then drop `start-xvfb.sh` and the `DISPLAY` override (the feature sets its own, typically `:1`,
and serves noVNC on port 6080). The Part 1 packages are **still required** — the feature ships
no Mesa/Vulkan drivers.

Pros: you can watch and interact; works on any host OS.
Cons: an extra feature and an exposed port.

### Option C — Forward the host X server

The host here is native Linux, so host display forwarding works directly and real windows
appear on your desktop.

```json
  "containerEnv": {
    "DISPLAY": "${localEnv:DISPLAY}",
    "XDG_RUNTIME_DIR": "/tmp/xdg-runtime",
    "XDG_CURRENT_DESKTOP": "GNOME"
  },
  "mounts": [
    "source=/tmp/.X11-unix,target=/tmp/.X11-unix,type=bind"
  ]
```

Requires `xhost +local:` on the host once per login. On a Wayland-only session this goes
through XWayland, which is normally present.

Pros: best fidelity and performance; native windows.
Cons: host-specific; relaxes host X access control; breaks if you work remotely.

---

## Part 3 — Running an app

With Option A (Xvfb):

```bash
export DISPLAY=:99
cargo run --example google_eframe
```

For file dialogs to work, wrap in a DBus session (see Part 4):

```bash
dbus-run-session -- cargo run --example google_eframe
```

Agent-style capture-and-drive loop:

```bash
export DISPLAY=:99
dbus-run-session -- ./target/debug/examples/google_eframe &
sleep 5
xwininfo -root -tree | grep -i excel     # confirm the window exists
import -window root /tmp/shot.png        # screenshot
xdotool mousemove 180 118 click 1        # click a widget
```

### Gotcha: keyboard input needs explicit focus

Xvfb runs with no window manager, so X input focus is never assigned to the application window.
Mouse clicks work, but **typed keys go nowhere** — the widget shows a focus ring and silently
ignores input, which looks exactly like an application bug.

Set focus once, explicitly, before typing:

```bash
WID=$(xdotool search --name "Excel Metric Processor" | head -1)
xdotool windowfocus "$WID"
xdotool mousemove 492 159 click 1
xdotool key ctrl+a
xdotool type --delay 80 "99:99:99"
```

The alternative is to install a minimal window manager (`openbox` or `fluxbox`) and start it
against `:99`, which assigns focus automatically. Option B (`desktop-lite`) includes one, so
this gotcha does not apply there.

Software rendering is slow. It is fine for screenshots and click-through verification, but not
for judging animation smoothness or frame rate.

---

## Part 4 — File dialogs (`rfd`)

`rfd` 0.17 defaults to the `xdg-portal` backend. With no portal running, dialogs fail
**silently** — clicking "Select Excel File..." does nothing at all, with no error and no panic.
This is easy to misdiagnose as an application bug.

The packages in Part 1 fix it with no code change, provided the app runs under a DBus session:

```bash
XDG_CURRENT_DESKTOP=GNOME dbus-run-session -- cargo run --example google_eframe
```

Verified: the GTK file chooser opens with the `.xlsx/.xls/.xlsm` filter applied. A harmless
warning is logged and can be ignored:

```
WARNING **: Unhandled parent window type
WARNING **: Failed to associate portal window with parent window
```

**Alternative, not recommended here:** switch `rfd` to its GTK3 backend
(`rfd = { version = "0.17.2", default-features = false, features = ["gtk3"] }`), which needs no
portal or DBus but does require `libgtk-3-dev` at build time. It avoids the DBus wrapper at the
cost of a Cargo.toml change and heavier build dependencies. The portal route was chosen above
because it keeps `Cargo.toml` untouched.

---

## Summary of files to change

| File | Change | Needed for |
|---|---|---|
| `.devcontainer/setup.sh` | Append the apt block (Part 1) | All options |
| `.devcontainer/devcontainer.json` | `containerEnv` + `postStartCommand` | Option A |
| `.devcontainer/start-xvfb.sh` | New file | Option A |
| `.devcontainer/devcontainer.json` | `desktop-lite` feature | Option B |
| `.devcontainer/devcontainer.json` | `DISPLAY` from `localEnv` + X11 socket mount | Option C |
