#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Photographs the road sites of a built map in the game: a test server and the headless client, both only
# inside headless sway, a helper mod that moves a static camera, one eye-level and one overhead shot per
# site. Never run by `make check` or CI: it needs the game.
#
#   tools/ingame-check/run.sh <build dir> <sites.json> <out dir> [site id ...]
#
# <build dir> is a project's build/ folder (it holds @naves_<world>/ and missions/dayzOffline.<world>/);
# sites.json comes from `cargo run --release -p naves-export --example road-sites`; <out dir> must be below
# /var/tmp or /tmp and gets the shots, index.md, contact sheets (sheet1.png ...), the server tree and logs.
# The same sites.json gives the same sites, so a later run is comparable with an earlier one.
#
# What it never does: edit, replace or delete a file of the player's settings (it only compares
# Documents/DayZ/steamuser_settings.DayZProfile before and after and fails on any difference but the
# lastMP* lines the game writes by itself); pass -name= or -profiles= to the client; start a game
# while one is running; use the default ports. Its own files in the client's profile folder (nc_*.txt)
# are removed again, and so is every symlink it made in the DayZ install. Client, server and sway are
# stopped when it ends, also on failure.
#
# Needs: dayzmake 2.0.0 or newer, sway, grim, ImageMagick (montage), python3, a DayZ client (app 221100),
# the DayZ Server (app 223350) and Proton as for dayz-client-headless.sh of the dayz-dev skill.
# Environment: STEAM_ROOT (default ~/.steam/debian-installation), DAYZ_DEV_SCRIPTS (the dayz-dev skill's
# scripts folder, default ~/.claude/skills/dayz-dev/scripts), SHOT_CROP (grim region, see drive.py),
# MIN_FREE_GB (free memory needed to start, default 20).
set -euo pipefail

die() { echo "error: $*" >&2; exit 1; }

[ $# -ge 3 ] || die "usage: $0 <build dir> <sites.json> <out dir> [site id ...]"
BUILD=$(cd "$1" && pwd -P) || die "no build dir $1"
SITES=$(readlink -f "$2")
[ -f "$SITES" ] || die "no sites file $2"
mkdir -p "$3"
OUT=$(cd "$3" && pwd -P)
shift 3
case "$OUT" in /var/tmp/* | /tmp/*) ;; *) die "the out dir must be below /var/tmp or /tmp, not $OUT" ;; esac

HERE=$(cd "$(dirname "$0")" && pwd -P)
STEAM_ROOT=${STEAM_ROOT:-$HOME/.steam/debian-installation}
SCRIPTS=${DAYZ_DEV_SCRIPTS:-$HOME/.claude/skills/dayz-dev/scripts}
INSTALL=$STEAM_ROOT/steamapps/common/DayZ
PFX=$STEAM_ROOT/steamapps/compatdata/221100/pfx/drive_c/users/steamuser
DOCS=$PFX/Documents/DayZ
CLIENT_PROFILE=$PFX/AppData/Local/DayZ
SETTINGS=steamuser_settings.DayZProfile

for tool in dayzmake sway grim montage python3 free; do
  command -v "$tool" >/dev/null || die "$tool not found"
done
for s in free-ports.sh make-server-tree.sh dayz-client-headless.sh find-dayzserver.sh; do
  [ -x "$SCRIPTS/$s" ] || die "$SCRIPTS/$s not found: install the dayz-dev skill or set DAYZ_DEV_SCRIPTS"
done
[ -f "$DOCS/$SETTINGS" ] || die "no $DOCS/$SETTINGS: has the game been started once?"
[ -d "$CLIENT_PROFILE" ] || die "no client profile folder $CLIENT_PROFILE"

MODS=("$BUILD"/@naves_*)
[ ${#MODS[@]} -eq 1 ] && [ -d "${MODS[0]}" ] || die "$BUILD must hold exactly one @naves_<world> folder"
MOD_NAME=$(basename "${MODS[0]}")
WORLD=${MOD_NAME#@naves_}
MISSION=$BUILD/missions/dayzOffline.$WORLD
[ -f "$MISSION/init.c" ] || die "no mission $MISSION"

# the game needs memory; one game at a time (the client script refuses a second one itself)
avail=$(free -g | awk '/^Mem:/ {print $7}')
[ "$avail" -ge "${MIN_FREE_GB:-20}" ] || die "only $avail GB of memory available (MIN_FREE_GB=${MIN_FREE_GB:-20})"
"$SCRIPTS/dayz-client-headless.sh" status >/dev/null || die "a DayZ client is already running; one game at a time"
for f in nc_cam.txt nc_cam.txt.tmp nc_ack.txt; do
  [ ! -e "$CLIENT_PROFILE/$f" ] || die "$CLIENT_PROFILE/$f exists and is not ours; look at it before running"
done

TREE=$OUT/server
STATE=$OUT/client-state
CAM=$OUT/cam
LINKS=()
SERVER_PID=

# --- the player's settings: copy before, compare after -------------------------------------------------
rm -rf "$OUT/documents-before"
mkdir -p "$OUT/documents-before"
for f in "$DOCS"/*; do [ -f "$f" ] && cp -p "$f" "$OUT/documents-before/"; done

check_settings() {
  local changed=0 f
  # the game writes lastMPServer, lastMPServerName and lastMPMission itself when it connects
  if ! diff <(grep -v '^lastMP' "$OUT/documents-before/$SETTINGS") <(grep -v '^lastMP' "$DOCS/$SETTINGS") \
    >"$OUT/settings.diff"; then
    echo "ERROR: the player's settings changed, see $OUT/settings.diff" >&2
    cat "$OUT/settings.diff" >&2
    changed=1
  fi
  for f in "$OUT"/documents-before/*; do
    [ "$(basename "$f")" = "$SETTINGS" ] && continue
    cmp -s "$f" "$DOCS/$(basename "$f")" || echo "note: the game itself rewrote $(basename "$f")" >&2
  done
  return $changed
}

# --- clean up: always ----------------------------------------------------------------------------------
cleanup() {
  local status=$? l pid
  trap - EXIT INT TERM
  set +e
  "$SCRIPTS/dayz-client-headless.sh" stop "$STATE" 2>/dev/null
  if [ -n "$SERVER_PID" ]; then
    kill -- "-$SERVER_PID" 2>/dev/null || kill "$SERVER_PID" 2>/dev/null
    for _ in $(seq 1 20); do kill -0 "$SERVER_PID" 2>/dev/null || break; sleep 1; done
    kill -9 -- "-$SERVER_PID" 2>/dev/null || kill -9 "$SERVER_PID" 2>/dev/null
  fi
  for _ in $(seq 1 30); do pgrep -f '[D]ayZ_x64\.exe' >/dev/null || break; sleep 1; done
  for pid in $(pgrep -f '[D]ayZ_x64\.exe'); do echo "warning: the client is still running, killing $pid" >&2; kill -9 "$pid"; done
  for l in "${LINKS[@]}"; do [ -L "$l" ] && rm -f "$l"; done
  rm -f "$CLIENT_PROFILE/nc_cam.txt" "$CLIENT_PROFILE/nc_cam.txt.tmp" "$CLIENT_PROFILE/nc_ack.txt"
  check_settings || status=1
  exit $status
}
trap cleanup EXIT INT TERM

# --- the helper mod, signed with a key made here -------------------------------------------------------
rm -rf "$CAM"
mkdir -p "$CAM/@NavesCam/addons" "$CAM/@NavesCam/keys"
(cd "$CAM" && dayzmake keygen -f NavesCam >/dev/null)
cp "$CAM/NavesCam.bikey" "$CAM/@NavesCam/keys/"
dayzmake build -f -k "$CAM/NavesCam.biprivatekey" "$HERE/NavesCam" "$CAM/@NavesCam/addons/NavesCam.pbo" >"$OUT/cam-build.log" 2>&1 \
  || { cat "$OUT/cam-build.log" >&2; die "the helper mod did not build"; }

# --- the server tree -----------------------------------------------------------------------------------
read -r GAME QUERY _ < <("$SCRIPTS/free-ports.sh" 3)
echo "ports: game $GAME, query $QUERY"
SERVER_SRC=$("$SCRIPTS/find-dayzserver.sh")
"$SCRIPTS/make-server-tree.sh" "$TREE" "$SERVER_SRC" >/dev/null
ln -sfn "$BUILD/$MOD_NAME" "$TREE/$MOD_NAME"
ln -sfn "$CAM/@NavesCam" "$TREE/@NavesCam"
cp "$BUILD/$MOD_NAME"/keys/*.bikey "$CAM/NavesCam.bikey" "$TREE/keys/"
rm -rf "${TREE:?}/mpmissions/dayzOffline.$WORLD"
cp -r "$MISSION" "$TREE/mpmissions/dayzOffline.$WORLD"

# noon and clear weather in the server's copy of the mission (the build's own init.c stays as it is)
awk '
  /^void main\(\)/ { in_main = 1 }
  in_main && /^}/ {
    print "\tGetGame().GetWorld().SetDate(2026, 7, 17, 12, 0);"
    print "\tWeather weather = GetGame().GetWeather();"
    print "\tweather.MissionWeather(true);"
    print "\tweather.GetOvercast().SetLimits(0, 0);"
    print "\tweather.GetRain().SetLimits(0, 0);"
    print "\tweather.GetFog().SetLimits(0, 0);"
    print "\tweather.GetOvercast().Set(0, 0, 86400);"
    print "\tweather.GetRain().Set(0, 0, 86400);"
    print "\tweather.GetFog().Set(0, 0, 86400);"
    print "\tweather.SetDynVolFogDistanceDensity(0, 0);"
    print "\tweather.SetDynVolFogHeightDensity(0, 0);"
    in_main = 0
  }
  { print }
' "$MISSION/init.c" >"$TREE/mpmissions/dayzOffline.$WORLD/init.c"
grep -q 'MissionWeather' "$TREE/mpmissions/dayzOffline.$WORLD/init.c" || die "could not patch main() in $MISSION/init.c"

cat >"$TREE/serverDZ.cfg" <<CFG
hostname = "naves-ingame-check";
password = "";
passwordAdmin = "";
maxPlayers = 4;
verifySignatures = 2;
forceSameBuild = 1;
disableVoN = 1;
shardId = "123abc";
disablePersonalLight = 1;
lightingConfig = 0;
serverTime = "2026/7/17/12/0";
serverTimeAcceleration = 1;
serverNightTimeAcceleration = 1;
serverTimePersistent = 0;
guaranteedUpdates = 1;
instanceId = 1;
steamQueryPort = $QUERY;
BattlEye = 0;
storageAutoFix = 1;
class Missions
{
    class DayZ
    {
        template = "dayzOffline.$WORLD";
    };
};
CFG

# --- start the server ----------------------------------------------------------------------------------
(cd "$TREE" && ulimit -c 0 && exec setsid ./DayZServer -config=serverDZ.cfg -profiles=profiles \
  "-mod=$MOD_NAME;@NavesCam" -port="$GAME" -nosplash -nopause -dologs >"$OUT/server.log" 2>&1) &
SERVER_PID=$!
echo "server pid $SERVER_PID"
ready=0
for _ in $(seq 1 90); do
  kill -0 "$SERVER_PID" 2>/dev/null || die "the server stopped, see $OUT/server.log and $TREE/profiles"
  if grep -qs 'Module: Mission; loaded' "$TREE"/profiles/script_*.log; then ready=1; break; fi
  sleep 2
done
[ $ready -eq 1 ] || die "the server did not load the mission in 180 s, see $TREE/profiles"

# --- start the client, only inside headless sway -------------------------------------------------------
for m in "$MOD_NAME" @NavesCam; do
  target=$INSTALL/$m
  [ ! -e "$target" ] && [ ! -L "$target" ] || die "$target exists; remove it first (a leftover link?)"
  case $m in @NavesCam) ln -s "$CAM/@NavesCam" "$target" ;; *) ln -s "$BUILD/$m" "$target" ;; esac
  LINKS+=("$target")
done
rm -rf "$STATE"
"$SCRIPTS/dayz-client-headless.sh" start "$STATE" "-mod=$MOD_NAME;@NavesCam" -connect=127.0.0.1 -port="$GAME"
connected=0
for _ in $(seq 1 120); do
  if grep -qs 'has connected' "$TREE"/profiles/DayZServer_*.RPT; then connected=1; break; fi
  grep -qs 'kicked' "$TREE"/profiles/DayZServer_*.RPT && die "the server kicked the client, see $TREE/profiles"
  sleep 5
done
[ $connected -eq 1 ] || die "the client did not connect in 10 minutes, see $STATE and $CLIENT_PROFILE"

# --- the shots -----------------------------------------------------------------------------------------
cp "$SITES" "$OUT/sites.json"
python3 -I "$HERE/drive.py" "$SITES" "$OUT" "$TREE/profiles" "$CLIENT_PROFILE" "$STATE" "$@"
echo "done: $OUT/index.md, $OUT/sheet*.png"
