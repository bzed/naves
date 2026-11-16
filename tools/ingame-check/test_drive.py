# SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests of the index and the contact sheets of drive.py, with a fake montage on PATH."""
import os
import stat
import tempfile
import unittest

import drive


def site(i, category="slight-angle"):
    return {"id": i, "category": category, "x": 1.5, "z": 2.5, "yaw_deg": 90.0,
            "numbers": {"b": 2.0, "a": 1.0}, "note": "a | b"}


def rows(n):
    return [(site(i), [f"{i:02d}-x-eye.png", f"{i:02d}-x-top.png"]) for i in range(1, n + 1)]


class IndexAndSheets(unittest.TestCase):
    def test_the_index_has_one_row_per_site_with_sorted_numbers_and_its_shots(self):
        lines = drive.index_text(rows(2)).splitlines()
        table = [l for l in lines if l.startswith("| ") and not l.startswith("| id")]
        self.assertEqual(len(table), 2)
        self.assertIn("a=1.0; b=2.0", table[0])
        self.assertIn("a / b", table[0])  # a pipe in a note would break the table
        self.assertIn("shots/01-x-eye.png<br>shots/01-x-top.png", table[0])

    def test_the_same_rows_give_the_same_index(self):
        self.assertEqual(drive.index_text(rows(3)), drive.index_text(rows(3)))

    def test_six_sites_make_a_sheet_in_site_order(self):
        groups = drive.sheet_groups(rows(13))
        self.assertEqual([len(g) for g in groups], [12, 12, 2])
        self.assertEqual(groups[0][0], "01-x-eye.png")
        self.assertEqual(groups[2], ["13-x-eye.png", "13-x-top.png"])
        self.assertEqual(drive.sheet_groups([]), [])

    def test_sheets_calls_montage_once_per_group_and_names_the_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            fake = os.path.join(tmp, "bin")
            os.mkdir(fake)
            script = os.path.join(fake, "montage")
            with open(script, "w") as f:
                f.write('#!/bin/sh\nfor a; do last="$a"; done\n: > "$last"\n')
            os.chmod(script, os.stat(script).st_mode | stat.S_IXUSR)
            old = os.environ["PATH"]
            os.environ["PATH"] = fake + os.pathsep + old
            try:
                drive.sheets(tmp, os.path.join(tmp, "shots"), rows(7))
            finally:
                os.environ["PATH"] = old
            self.assertEqual(sorted(n for n in os.listdir(tmp) if n.startswith("sheet")),
                             ["sheet1.png", "sheet2.png"])


if __name__ == "__main__":
    unittest.main()
