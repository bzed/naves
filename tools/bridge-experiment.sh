#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Replayable form of the binarize bridge experiment (docs/formats/binarize-world.md).
#
#   tools/bridge-experiment.sh sandbox <dir>    create the sandbox (Binarize copy, Wine prefix, Z:/P: links)
#   tools/bridge-experiment.sh addon <dir>      write the flat test world, textures, rvmat and config into <dir>/P
#   tools/bridge-experiment.sh binarize <dir>   run binarize on the world; result in <dir>/out
#   tools/bridge-experiment.sh pack <dir>       pack <dir>/out + config into <dir>/mod/@bridgetest
#
# <dir> must be below /var/tmp or /tmp. Nothing is written into the Steam directories; vanilla files that
# binarize needs are extracted from the user's install into the sandbox at run time and are not kept in
# the repository. Needs: bzed's armake2 fork, DayZ Tools, Proton, ImageMagick (convert).
set -euo pipefail

STEAM="${STEAM_ROOT:-$HOME/.steam/steam}"
STEAM="$(readlink -f "$STEAM")"
TOOLS="${DAYZ_TOOLS:-$STEAM/steamapps/common/DayZ Tools}"
DAYZ="${DAYZ:-$STEAM/steamapps/common/DayZ}"
PROTON="${PROTON:-$STEAM/steamapps/common/Proton Hotfix/proton}"
WORLD=bridgetest
PREFIX="naves/worlds/$WORLD"
REPO="$(cd "$(dirname "$0")/.." && pwd)"

die() { echo "error: $*" >&2; exit 1; }

dir_arg() {
  [ -n "${1:-}" ] || die "usage: $0 <stage> <dir under /var/tmp or /tmp>"
  case "$1" in /var/tmp/* | /tmp/*) ;; *) die "dir must be below /var/tmp or /tmp" ;; esac
  mkdir -p "$1"
  W="$(readlink -f "$1")"
}

proton() {
  STEAM_COMPAT_CLIENT_INSTALL_PATH="$STEAM" STEAM_COMPAT_DATA_PATH="$W/prefix" SteamAppId=830640 \
    "$PROTON" run "$@"
}

stage_sandbox() {
  [ -d "$TOOLS/Bin/Binarize" ] || die "DayZ Tools not found at $TOOLS (set DAYZ_TOOLS)"
  [ -x "$PROTON" ] || die "Proton not found at $PROTON (set PROTON)"
  rm -rf "$W/Binarize"
  cp -r "$TOOLS/Bin/Binarize" "$W/Binarize"
  mkdir -p "$W/out" "$W/prefix" "$W/P" "$W/tex"
  proton cmd.exe /c exit || true # the first run creates the prefix
  ln -sfn "$W" "$W/prefix/pfx/dosdevices/z:"
  ln -sfn "$W/P" "$W/prefix/pfx/dosdevices/p:"
}

stage_addon() {
  command -v armake2 >/dev/null || die "armake2 not found (set PATH)"
  armake2 -h 2>&1 | grep -q -- --proton-binarize || die "armake2 is not bzed's fork"
  local A="$W/P/$PREFIX" L="$W/P/$PREFIX/data/layers"
  mkdir -p "$L"
  (cd "$REPO" && CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/naves-target}" \
    cargo run -q -p naves-formats --example flat_world -- "$A/$WORLD.wrp" "$WORLD")
  convert -size 512x512 xc:'#4a7a3a' "$L/s_000_000_lco.png"
  convert -size 512x512 xc:'#ff0000' -alpha off "$L/m_000_000_lca.png"
  armake2 img2paa -f -t dxt1 "$L/s_000_000_lco.png" "$L/s_000_000_lco.paa"
  armake2 img2paa -f -t dxt5 "$L/m_000_000_lca.png" "$L/m_000_000_lca.paa"
  rm -f "$L"/*.png
  cp "$REPO/tools/bridge-experiment/config.cpp" "$A/config.cpp"
  cp "$REPO/tools/bridge-experiment/p_000-000.rvmat" "$L/p_000-000.rvmat"
}

stage_binarize() {
  rm -rf "$W/out"
  mkdir -p "$W/out"
  cd "$W/P"
  proton cmd.exe /c 'Z:\Binarize\binarize.exe -always -silent=0 -textures=Z:\tex -binPath=P:\ P:\naves\worlds\bridgetest Z:\out > Z:\binarize.log 2>&1'
  ls -la "$W/out"
}

stage_pack() {
  local M="$W/mod/@$WORLD"
  rm -rf "$W/pack" "$M"
  mkdir -p "$W/pack/$PREFIX" "$M/addons"
  cp "$W/P/$PREFIX/config.cpp" "$W/pack/$PREFIX/config.cpp"
  cp -r "$W/P/$PREFIX/data" "$W/pack/$PREFIX/data"
  cp "$W/out/$WORLD.wrp" "$W/pack/$PREFIX/$WORLD.wrp"
  printf '%s' "${PREFIX//\//\\}" > "$W/pack/$PREFIX/\$PBOPREFIX\$"
  mkdir -p "$W/key" "$M/keys"
  [ -f "$W/key/naves_test.biprivatekey" ] || (cd "$W/key" && armake2 keygen naves_test)
  armake2 build -f -k "$W/key/naves_test.biprivatekey" "$W/pack/$PREFIX" "$M/addons/naves_$WORLD.pbo"
  cp "$W/key/naves_test.bikey" "$M/keys/"
  ls -la "$M/addons" "$M/keys"
}

[ $# -eq 2 ] || die "usage: $0 sandbox|addon|binarize|pack <dir>"
dir_arg "$2"
case "$1" in
  sandbox) stage_sandbox ;;
  addon) stage_addon ;;
  binarize) stage_binarize ;;
  pack) stage_pack ;;
  *) die "unknown stage $1" ;;
esac
