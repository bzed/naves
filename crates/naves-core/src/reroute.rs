// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Road planning: a bend tighter than the road pieces can build is re-routed across the terrain between two
//! anchors on its own road.
//!
//! The design is in `docs/superpowers/specs/2026-11-21-road-planning-design.md`. This module holds the named
//! constants of its constants table and [`RerouteParams`], which collects them so that tests can change them.
//! The parts are sub-modules: [`raster`] is the obstacle raster with its distance maps and wall lines, [`band`] the
//! feasibility band, the profile and the grade limit of a leg, [`field`] the cost-to-go field of a leg.

pub mod band;
pub mod field;
pub mod raster;

/// The switch: road planning runs only when this is on. Off until the result is judged.
pub const REROUTE_ENABLED: bool = false;
/// Octile and chamfer excess: a chamfer distance is up to this much longer than the straight one.
pub const CHAMFER_ERROR: f64 = 1.0824;
/// A two-stretch node that turns more than this (degrees) ends a chain.
pub const CHAIN_TURN_MAX_DEG: f64 = crate::smooth::CHAIN_TURN_MAX_DEG;
/// Spacing of the samples of a road for the exemption and the stamping, in metres.
pub const SAMPLE_M: f64 = crate::smooth::SAMPLE_M;
/// Minimum radii of road beside a tight run that belong to its zone.
pub const PLAN_LEAD: f64 = 2.0;
/// The planning radius is this much over the tightest arc.
pub const PLAN_RADIUS_MARGIN: f64 = 1.05;
/// A written route's tightest sample, as a fraction of the tightest arc, that counts as reached.
pub const PLAN_REACHED_FRACTION: f64 = 0.99;
/// Search step, in metres.
pub const PLAN_STEP_M: f64 = 6.0;
/// Largest heading change of a step, in degrees.
pub const PLAN_MAX_TURN_DEG: f64 = 45.0;
/// Straight at a junction anchor, in metres.
pub const PLAN_JUNCTION_STRAIGHT_M: f64 = 6.25;
/// Road held beside a bridge or tunnel end, in metres.
pub const PLAN_BRIDGE_HOLD_M: f64 = crate::smooth::WIDEN_BRIDGE_HOLD_M;
/// A deck or tunnel this near a fixed anchor belongs to it and is exempt there, in metres.
pub const PLAN_DECK_ANCHOR_M: f64 = 12.0;
/// Furthest a route strays from the original line, in minimum radii.
pub const PLAN_STRAY_MAX: f64 = 3.0;
/// Largest cut of the profile band, in metres.
pub const PLAN_CUT_M: f64 = 6.0;
/// Largest fill of the profile band, in metres.
pub const PLAN_FILL_M: f64 = 6.0;
/// Road height at an anchor must be within this of the terrain, in metres.
pub const PLAN_ANCHOR_TOL_M: f64 = 0.5;
/// Thirds of the gap to the grade ceiling that are tried in turn.
pub const PLAN_GRADE_RELAX_STEPS: usize = 3;
/// A planned leg this much steeper than drawn is counted.
pub const PLAN_STEEPER_MARGIN: f64 = 0.02;
/// Window of the reported steepest drawn grade, in metres.
pub const PLAN_DRAWN_WINDOW_M: f64 = 20.0;
/// Metres of road per metre of climb.
pub const W_SLOPE: f64 = 4.0;
/// Weight of the turn, times `|δ| · R`.
pub const W_CURVE: f64 = 0.1;
/// Metres of road per m² of earthwork.
pub const W_EARTH: f64 = 1.0;
/// Metres of road per m² between route and original line.
pub const W_STRAY: f64 = 0.25;
/// Weight and expanded states of each try of the search.
pub const PLAN_BUDGETS: [(f64, usize); 3] = [(1.0, 10_000), (1.5, 20_000), (3.0, 40_000)];
/// A longer zone is not planned, in metres.
pub const PLAN_MAX_ZONE_M: f64 = 400.0;
/// Exempt part of a road beyond where the original line comes near, in metres.
pub const PLAN_EXEMPT_EXTEND_M: f64 = 10.0;
/// Wall crossings this near an anchor are not counted, in metres.
pub const PLAN_CROSSING_END_M: f64 = 10.0;
/// Most cells of the obstacle raster of a leg; the resolution grows by 1.25 until it fits.
pub const PLAN_RASTER_MAX_CELLS: usize = 4_000_000;
/// Most cells of the cost-to-go field of a leg; the resolution grows by 1.25 until it fits.
pub const PLAN_FIELD_MAX_CELLS: usize = 1_000_000;

/// The constants of road planning as values, so that tests can change them. `Default` is the table.
#[derive(Debug, Clone, PartialEq)]
pub struct RerouteParams {
    /// See [`REROUTE_ENABLED`].
    pub enabled: bool,
    /// See [`CHAMFER_ERROR`].
    pub chamfer_error: f64,
    /// See [`PLAN_LEAD`].
    pub lead: f64,
    /// See [`PLAN_RADIUS_MARGIN`].
    pub radius_margin: f64,
    /// See [`PLAN_REACHED_FRACTION`].
    pub reached_fraction: f64,
    /// See [`PLAN_STEP_M`].
    pub step_m: f64,
    /// See [`PLAN_MAX_TURN_DEG`].
    pub max_turn_deg: f64,
    /// See [`PLAN_JUNCTION_STRAIGHT_M`].
    pub junction_straight_m: f64,
    /// See [`PLAN_BRIDGE_HOLD_M`].
    pub bridge_hold_m: f64,
    /// See [`PLAN_DECK_ANCHOR_M`].
    pub deck_anchor_m: f64,
    /// See [`PLAN_STRAY_MAX`].
    pub stray_max: f64,
    /// See [`PLAN_CUT_M`].
    pub cut_m: f64,
    /// See [`PLAN_FILL_M`].
    pub fill_m: f64,
    /// See [`PLAN_ANCHOR_TOL_M`].
    pub anchor_tol_m: f64,
    /// See [`PLAN_GRADE_RELAX_STEPS`].
    pub grade_relax_steps: usize,
    /// See [`PLAN_STEEPER_MARGIN`].
    pub steeper_margin: f64,
    /// See [`PLAN_DRAWN_WINDOW_M`].
    pub drawn_window_m: f64,
    /// See [`W_SLOPE`].
    pub w_slope: f64,
    /// See [`W_CURVE`].
    pub w_curve: f64,
    /// See [`W_EARTH`].
    pub w_earth: f64,
    /// See [`W_STRAY`].
    pub w_stray: f64,
    /// See [`PLAN_BUDGETS`].
    pub budgets: [(f64, usize); 3],
    /// See [`PLAN_MAX_ZONE_M`].
    pub max_zone_m: f64,
    /// See [`PLAN_EXEMPT_EXTEND_M`].
    pub exempt_extend_m: f64,
    /// See [`PLAN_CROSSING_END_M`].
    pub crossing_end_m: f64,
    /// See [`PLAN_RASTER_MAX_CELLS`].
    pub raster_max_cells: usize,
    /// See [`PLAN_FIELD_MAX_CELLS`].
    pub field_max_cells: usize,
}

impl Default for RerouteParams {
    fn default() -> Self {
        Self {
            enabled: REROUTE_ENABLED,
            chamfer_error: CHAMFER_ERROR,
            lead: PLAN_LEAD,
            radius_margin: PLAN_RADIUS_MARGIN,
            reached_fraction: PLAN_REACHED_FRACTION,
            step_m: PLAN_STEP_M,
            max_turn_deg: PLAN_MAX_TURN_DEG,
            junction_straight_m: PLAN_JUNCTION_STRAIGHT_M,
            bridge_hold_m: PLAN_BRIDGE_HOLD_M,
            deck_anchor_m: PLAN_DECK_ANCHOR_M,
            stray_max: PLAN_STRAY_MAX,
            cut_m: PLAN_CUT_M,
            fill_m: PLAN_FILL_M,
            anchor_tol_m: PLAN_ANCHOR_TOL_M,
            grade_relax_steps: PLAN_GRADE_RELAX_STEPS,
            steeper_margin: PLAN_STEEPER_MARGIN,
            drawn_window_m: PLAN_DRAWN_WINDOW_M,
            w_slope: W_SLOPE,
            w_curve: W_CURVE,
            w_earth: W_EARTH,
            w_stray: W_STRAY,
            budgets: PLAN_BUDGETS,
            max_zone_m: PLAN_MAX_ZONE_M,
            exempt_extend_m: PLAN_EXEMPT_EXTEND_M,
            crossing_end_m: PLAN_CROSSING_END_M,
            raster_max_cells: PLAN_RASTER_MAX_CELLS,
            field_max_cells: PLAN_FIELD_MAX_CELLS,
        }
    }
}
