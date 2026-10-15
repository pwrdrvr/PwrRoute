#!/bin/sh
# What GPUI links against on Linux: windowing (Wayland, X11), input, fonts and
# the Vulkan loader. The list comes from disktree, which ships the same
# gpui-kit. Used by CI and release; run it on a fresh Debian/Ubuntu box too.
set -eu
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  pkg-config libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev \
  libvulkan-dev libasound2-dev libssl-dev libzstd-dev
