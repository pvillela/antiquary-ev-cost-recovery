# .devcontainer/Dockerfile

# Pinned to jammy (Ubuntu 22.04) deliberately, not merely inherited: it matches the host and the
# `ubuntu-22.04` runner in .github/workflows/release-build.yaml, so all three agree on glibc 2.35
# and a binary built here behaves like the one CI ships. A newer base would build against a newer
# glibc and reintroduce the "version `GLIBC_2.39' not found" failure on 22.04 machines.
FROM mcr.microsoft.com/devcontainers/base:jammy

# GUI runtime for the eframe/egui app, and the tooling to drive and observe it headlessly.
#
# This lives here rather than in setup.sh so it becomes a cached image layer. Docker keys its
# build cache on the instructions and base image, not on the workspace path, whereas the
# devcontainer's own identity is a hash of that path -- so moving the project rebuilds the
# container but reuses this layer, instead of re-downloading a few hundred MB over the network.
#
# The package set is copied verbatim from docs/Devcontainer_GUI_Options.md, where it was verified
# working on 2026-07-26, with `openbox` added (see start-xvfb.sh). `--no-install-recommends` is
# deliberately NOT used: the portal packages below pull working defaults through recommends, and
# trimming them is exactly the kind of change that turns into a silently broken file dialog.
RUN export DEBIAN_FRONTEND=noninteractive \
    && apt-get update -qq \
    && apt-get install -y -qq \
        xvfb x11-utils xdotool imagemagick \
        openbox \
        libx11-6 libxcb1 libxkbcommon-x11-0 libxcursor1 libxrandr2 libxi6 libwayland-client0 \
        libgl1 libegl1 libgl1-mesa-dri libvulkan1 mesa-vulkan-drivers \
        fonts-dejavu-core \
        dbus-x11 xdg-desktop-portal xdg-desktop-portal-gtk \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*
