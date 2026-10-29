#!/usr/bin/env bash
# Runs PwrRoute's Linux checks inside a Sandboxer sandbox (Ubuntu x64), so
# the Rust builds, clippy and cargo-about run there instead of on the
# laptop. The procedure and the transfer steps are in
# .agents/skills/sandboxer-verification/SKILL.md.
#
#   scripts/sandboxer/verify.sh setup      apt packages, rustup + the pinned toolchain,
#                                          Node 22+, CI's cargo-deny and cargo-about, cargo fetch
#   scripts/sandboxer/verify.sh lint       cargo xtask lint (CI's Typecheck job)
#   scripts/sandboxer/verify.sh test [args]
#                                          cargo xtask test (CI's Test job); with args,
#                                          cargo test --locked ARGS instead
#   scripts/sandboxer/verify.sh build      cargo build --release (CI's Build job)
#   scripts/sandboxer/verify.sh e2e [args] cargo xtask e2e under Xvfb (CI's Linux Desktop E2E)
#   scripts/sandboxer/verify.sh licenses   cargo xtask licenses: regenerates THIRD_PARTY_LICENSES
#   scripts/sandboxer/verify.sh all        setup, lint, test, e2e; runs every step
#
# Each invocation starts a new .local/sandboxer/status. Each step writes
# .local/sandboxer/<step>.log and appends "<step> exit=<code> secs=<seconds>".
# A step longer than the sandbox exec limit must run under nohup; poll status.
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root" || exit 1
out_dir="$repo_root/.local/sandboxer"
mkdir -p "$out_dir"

# One run at a time: every step shares target/ and the status file.
exec 9> "$out_dir/lock"
if ! flock -n 9; then
  echo "another verify.sh is running in $repo_root" >&2
  exit 3
fi

# Installed only when the image's Node is older than 22.
node_version="v22.21.1"
node_dir="/opt/node-$node_version"
export PATH="$HOME/.cargo/bin:$node_dir/bin:$PATH"
# As CI: no debug info. Nobody reads it here, and it is a large share of
# compile and link time and of target/'s size.
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_TERM_COLOR=never
# CI checks run without a GitHub token, and none needs one. Sandboxes are
# created with noEnv, so this is a second guard.
unset GH_TOKEN GITHUB_TOKEN

# GPUI's libraries (scripts/ci/install-linux-deps.sh), a C toolchain, and the
# headless desktop ci.yml's e2e-linux job installs.
apt_packages=(
  build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev
  libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev
  libvulkan-dev libasound2-dev libssl-dev libzstd-dev
  xvfb xauth mesa-vulkan-drivers x11-utils imagemagick xdg-utils
)

# The release binaries CI's taiki-e/install-action pin installs (see the
# Typecheck job's "Install cargo-deny and cargo-about" log). cargo-about's
# output changes between releases, so a different version fails the
# THIRD_PARTY_LICENSES check; move these when that pin moves.
cargo_deny_version="0.20.2"
cargo_about_version="0.9.2"

install_tool() {
  local tool="$1" version="$2"
  [[ "$("$tool" -V 2>/dev/null)" == "$tool $version" ]] && return 0
  local name="$tool-$version-x86_64-unknown-linux-musl"
  local url="https://github.com/EmbarkStudios/$tool/releases/download/$version/$name.tar.gz"
  local dir
  dir="$(mktemp -d)" || return 1
  curl -fsSL "$url" -o "$dir/$name.tar.gz" &&
    curl -fsSL "$url.sha256" -o "$dir/$name.tar.gz.sha256" &&
    (cd "$dir" && sha256sum -c --quiet <(echo "$(cut -d' ' -f1 "$name.tar.gz.sha256")  $name.tar.gz")) &&
    tar -xzf "$dir/$name.tar.gz" -C "$dir" &&
    install -m 755 "$dir/$name/$tool" "$HOME/.cargo/bin/$tool"
  local code=$?
  rm -rf "$dir"
  return "$code"
}

setup() {
  if ! dpkg -s "${apt_packages[@]}" > /dev/null 2>&1; then
    sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq || return 1
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -qq -y --no-install-recommends \
      "${apt_packages[@]}" || return 1
  fi
  # The image's Node does when it is 22 or newer, as xtask lint needs.
  local node_major
  node_major="$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || echo 0)"
  if ((node_major < 22)); then
    local archive="node-$node_version-linux-x64.tar.xz"
    curl -fsSL "https://nodejs.org/dist/$node_version/$archive" -o "/tmp/$archive" || return 1
    sudo mkdir -p "$node_dir" || return 1
    sudo tar -xJf "/tmp/$archive" -C "$node_dir" --strip-components=1 || return 1
  fi
  if ! command -v rustup > /dev/null; then
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none || return 1
  fi
  # Installs rust-toolchain.toml's channel and components.
  rustup toolchain install || return 1
  install_tool cargo-deny "$cargo_deny_version" || return 1
  install_tool cargo-about "$cargo_about_version" || return 1
  # A copy outside the checkout, so it survives checking out an older commit.
  mkdir -p "$HOME/pwrroute/bin"
  cp scripts/sandboxer/transfer.py "$HOME/pwrroute/bin/transfer.py" || return 1
  node -v && cargo -V && cargo deny -V && cargo about -V && cargo fetch --locked
}

run_step() {
  local step="$1"
  shift
  local log="$out_dir/$step.log"
  local start
  start="$(date +%s)"
  case "$step" in
    setup) setup ;;
    lint) cargo xtask lint ;;
    # xtask's test task takes no arguments, so a filtered run calls cargo.
    test) if (($#)); then cargo test --locked "$@"; else cargo xtask test; fi ;;
    build) cargo build --release --locked --workspace ;;
    # Same display as CI. PWRROUTE_E2E_DISPOSABLE stays unset, so the
    # scenarios that make PwrRoute the default browser skip (e2e/AGENTS.md).
    e2e) xvfb-run --auto-servernum --server-args="-screen 0 1920x1080x24" cargo xtask e2e "$@" ;;
    licenses) cargo xtask licenses ;;
  esac > "$log" 2>&1
  local code=$?
  echo "$step exit=$code secs=$(($(date +%s) - start))" | tee -a "$out_dir/status"
  return "$code"
}

step="${1:-}"
[[ $# -gt 0 ]] && shift
case "$step" in
  setup | lint | test | build | e2e | licenses)
    : > "$out_dir/status"
    run_step "$step" "$@"
    ;;
  all)
    : > "$out_dir/status"
    run_step setup || exit 1
    failed=0
    run_step lint || failed=1
    run_step test || failed=1
    run_step e2e || failed=1
    echo "all exit=$failed" | tee -a "$out_dir/status"
    exit "$failed"
    ;;
  *)
    sed -n '2,19p' "$0" >&2
    exit 2
    ;;
esac
