#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Moves the camera of the NavesCam helper mod through the sites and photographs each one.

Called by run.sh once the client is connected; it does not start or stop anything. For each site of
sites.json the server teleports the player (nc_site.txt in the server's profile folder) and the client
puts its static camera (nc_cam.txt in the client's profile folder) first at eye level, then overhead.
Every command carries a number that the mod repeats in its acknowledgement file.

usage: drive.py <sites.json> <out dir> <server profile> <client profile> <client state> [site id ...]
"""
import json
import math
import os
import subprocess
import sys
import time

EYE_M = 1.7  # camera height above the road surface for the eye-level shot
TOP_M = 22.0  # and for the overhead shot, which looks 45 degrees down from as far back
BACK_M = 12.0  # distance before the site for the eye-level shot
BRIDGE_BACK_M = 30.0
CROP = os.environ.get("SHOT_CROP", "3,29 1274x688")  # the game's window inside the headless output
SHEET = 6  # sites per contact sheet


def write(path, text):
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        f.write(text + "\n")
    os.replace(tmp, path)


def wait_ack(path, seq, timeout=40):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            with open(path) as f:
                line = f.read().strip()
            if line.split()[0] == str(seq):
                return line
        except (OSError, IndexError):
            pass
        time.sleep(0.3)
    return None


def main():
    sites_file, out, server_prof, client_prof, state, *ids = sys.argv[1:]
    with open(sites_file) as f:
        sites = json.load(f)
    wanted = {int(i) for i in ids}
    sites = [s for s in sites if not wanted or s["id"] in wanted]
    shots = os.path.join(out, "shots")
    os.makedirs(shots, exist_ok=True)
    with open(os.path.join(state, "wayland-display")) as f:
        display = f.read().strip()
    env = dict(os.environ, WAYLAND_DISPLAY=display)
    seq = int(time.time()) % 100000 * 10
    first = True
    rows = []
    for s in sites:
        back = BRIDGE_BACK_M if s["category"].startswith("bridge") else BACK_M
        yaw = s["yaw_deg"]
        fx, fz = math.sin(math.radians(yaw)), math.cos(math.radians(yaw))
        # the player stands 4 m behind the camera, which keeps the area around the site loaded
        seq += 1
        write(os.path.join(server_prof, "nc_site.txt"),
              f"{seq} {s['x'] - fx * (back + 4):.1f} {s['z'] - fz * (back + 4):.1f}")
        if wait_ack(os.path.join(server_prof, "nc_site_ack.txt"), seq) is None:
            sys.exit(f"site {s['id']}: the server did not acknowledge the teleport")
        name = f"{s['id']:02d}-{s['category']}"
        files = []
        for kind, dy, bk, pitch in (("eye", EYE_M, back, -2.0), ("top", TOP_M, TOP_M, None)):
            if pitch is None:
                pitch = -math.degrees(math.atan2(dy, bk))
            seq += 1
            write(os.path.join(client_prof, "nc_cam.txt"),
                  f"{seq} 1 {s['x'] - fx * bk:.1f} {s['z'] - fz * bk:.1f} {dy} {yaw:.1f} {pitch:.1f} 1.0")
            if wait_ack(os.path.join(client_prof, "nc_ack.txt"), seq) is None:
                sys.exit(f"site {s['id']}: the client did not acknowledge the camera")
            # the world around a new position streams in; the first one takes longest
            time.sleep((25 if first else 9) if kind == "eye" else 4)
            first = False
            file = f"{name}-{kind}.png"
            subprocess.run(["grim", "-g", CROP, os.path.join(shots, file)], env=env, check=True)
            files.append(file)
        print(f"site {s['id']} {s['category']}: {', '.join(files)}", flush=True)
        rows.append((s, files))
    seq += 1
    write(os.path.join(client_prof, "nc_cam.txt"), f"{seq} 0 0 0 0 0 0 0")
    index(out, rows)
    sheets(out, shots, rows)


def index_text(rows):
    """The text of index.md for rows of (site, [shot file names])."""
    lines = [
        "# In-game road check",
        "",
        "`eye`: 1.7 m above the road, looking along the site's direction. `top`: 22 m up, 45 degrees down.",
        "Positions are map metres (x east, z north); the yaw is degrees clockwise from north.",
        "",
        "| id | category | x | z | yaw | numbers | note | shots |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for s, files in rows:
        numbers = "; ".join(f"{k}={v}" for k, v in sorted(s["numbers"].items()))
        shots = "<br>".join(f"shots/{f}" for f in files)
        note = s["note"].replace("|", "/")
        lines.append(f"| {s['id']} | {s['category']} | {s['x']} | {s['z']} | {s['yaw_deg']} "
                     f"| {numbers} | {note} | {shots} |")
    return "\n".join(lines) + "\n"


def index(out, rows):
    with open(os.path.join(out, "index.md"), "w") as f:
        f.write(index_text(rows))


def sheet_groups(rows):
    """The shot file names of each contact sheet: SHEET sites each, in site order."""
    return [[f for _, fs in rows[n:n + SHEET] for f in fs] for n in range(0, len(rows), SHEET)]


def sheets(out, shots, rows):
    for n, files in enumerate(sheet_groups(rows), start=1):
        target = os.path.join(out, f"sheet{n}.png")
        subprocess.run(["montage", "-tile", "2x", "-geometry", "640x344+4+4", "-set", "label", "%t",
                        *[os.path.join(shots, f) for f in files], target], check=True)


if __name__ == "__main__":
    main()
