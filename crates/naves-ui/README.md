# naves-ui

<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

The design system of the Náves GUI: Rerun's `re_ui`, vendored as a standalone crate.

Vendored from [`rerun_ui_kit` 0.38.1-base](https://github.com/jiangheng90-opensource/rerun_ui_kit)
(extraction of [rerun](https://github.com/rerun-io/rerun)'s `re_ui`, MIT OR Apache-2.0), with the
rerun data stack removed so it depends only on egui and small pure-Rust crates. See
[`PORTING.md`](PORTING.md) for the porting ledger: what was changed against upstream, and how to
re-port when a newer `re_ui` is wanted. The diff against upstream **is** the ledger — keep it small
and boring.

This crate is vendored third-party code: it does not opt into the workspace's clippy set (that
would grow the ledger and make re-ports painful); `unsafe_code` stays forbidden. It is held to its
upstream tests plus tests for our modifications, not to the workspace coverage floor.

The bundled Inter font is under the SIL Open Font License (`data/OFL.txt`).
