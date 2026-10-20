#!/bin/sh
# Uninstall every rustup toolchain except the one the workspace pins, so the
# Swatinem/rust-cache key depends on rust-toolchain.toml alone.
#
# rust-cache hashes the version of every toolchain `rustup toolchain list`
# shows, not only the active one. GitHub's runner images ship a preinstalled
# stable toolchain whose version moves with each image rollout, and rollouts
# are gradual: two runs minutes apart can land on images with different
# stables (1.99.0 and 1.99.1 on ubuntu-24.04 images 20261012 and 20261018)
# and so compute different keys. Only main saves the cache, so a PR that
# lands on an image main has not run on yet finds nothing to restore and
# compiles cold. Run this after the toolchain is installed and before
# rust-cache, from the workspace root (the rust-toolchain.toml override
# decides which toolchain is pinned).
set -eu
pinned=$(rustc -vV)
for toolchain in $(rustup toolchain list | cut -d' ' -f1 | tr -d '\r'); do
  if [ "$(rustup run "$toolchain" rustc -vV)" = "$pinned" ]; then
    echo "keeping $toolchain"
  else
    rustup toolchain uninstall "$toolchain"
  fi
done
rustup toolchain list
