// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Merging overlapping imagery into one picture of the map.
//!
//! Images of one area come from different sources and flights: other exposure, other season, other
//! sharpness. Everything is measured on a coarse grid of one cell per land cell, which is fine enough for the
//! fall-off across an aerial photo and coarse enough that a car or a roof does not count:
//!
//! 1. every raster gets a mean colour per cell;
//! 2. the raster with the most cells is the reference, its colours stay as they are;
//! 3. the others are matched to the brightness seen so far, largest overlap first, with a smooth gain per cell, the
//!    same in all three channels (so exposure and fall-off are evened out, the colours and the detail are left alone);
//! 4. the sharpest raster is drawn on top, equals in the order of the list;
//! 5. a raster fades in over a few cells at its edges.

use rayon::prelude::*;

use super::{ImageryError, ImageryRaster, RasterSampler, real_px_m};
use crate::heightmap::MapGeo;
use crate::project::Project;

/// Samples per cell side with which the mean colour of a cell is taken.
const CELL_SAMPLES: u32 = 4;

/// A channel below this (of 255) in either image is too dark to tell a gain from noise; such cells do not
/// take part in the colour matching (deep water and shadows differ in colour without meaning anything).
const DARK_FLOOR: f32 = 8.0;

/// The colour of a raster is never changed by more than this factor (smallest, largest). Satellite scenes are
/// much darker than aerial photographs of the same ground (a Sentinel-2 true colour image averages 44, 52, 32 where
/// the Bavarian orthophotos average 93, 105, 94, so the gain is 0.3 to 0.5), and a difference this large is still an
/// exposure; a larger one is more likely a different scene.
const GAIN_LIMITS: (f32, f32) = (0.15, 6.0);

/// A cell whose gain is further than this factor from the raster's median gain, in either direction (a cell
/// gain above `1 + CONTENT_CHANGE` times the median or below its inverse), is real change (a mown field,
/// snow, a new estate) and not exposure; it takes the median so that the change is not stamped into the
/// correction. Sensors do not only differ in exposure: their colours differ by the kind of ground too (forest is
/// nearly black in a satellite scene and not in a photograph), so the value is generous: 1.0 is a factor of 2.
/// Tuned on Berchtesgaden, Sentinel-2 under Bavarian orthophotos: the mean difference of the matched colours from
/// the satellite's per 100 m cell went from 19 to 15 levels (0.25 with the old limits: 19; 1.5: 13, but then a
/// block 3 times as bright is no longer recognised in `changed_content_leaves_no_stamp`).
const CONTENT_CHANGE: f32 = 1.0;

/// Fewer overlapping cells than this and the colours are not matched at all.
const MIN_OVERLAP_CELLS: usize = 16;

/// The gain field is smoothed with a box blur of this radius in cells, applied [`BLUR_PASSES`] times.
const BLUR_RADIUS: usize = 2;

/// Number of blur passes.
const BLUR_PASSES: usize = 2;

/// A raster fades in over this many cells from its edges.
const FEATHER_CELLS: f32 = 4.0;

/// Sources whose pixel sizes differ by less than this factor count as equally sharp.
const SAME_SHARPNESS: f64 = 1.05;

/// Largest picture `Composite::paint` makes in one call.
const MAX_PAINT_PIXELS: u64 = 1 << 28;

/// A rectangle of the map in metres, as a grid of pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapRect {
    /// Western edge (x of the left side of column 0).
    pub west: f64,
    /// Northern edge (y of the top of row 0).
    pub north: f64,
    /// Width of a pixel in metres.
    pub step_x: f64,
    /// Height of a pixel in metres.
    pub step_y: f64,
    /// Pixels per row.
    pub width: u32,
    /// Rows.
    pub height: u32,
}

/// What the merge found about one source.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceReport {
    /// The name of the raster (its file).
    pub label: String,
    /// Share of the map's cells that the raster covers completely.
    pub coverage: f64,
    /// Share of those cells that other images covered as well.
    pub overlap: f64,
    /// The typical correction of the brightness, the same in all three channels: 1 means untouched.
    pub median_gain: [f32; 3],
    /// Share of the overlap that looked like real change and not like exposure.
    pub changed_share: f64,
}

/// What the merge found.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeReport {
    /// The raster whose colours were kept, as an index into the input.
    pub reference: Option<usize>,
    /// One entry per input raster, in the order of the input.
    pub sources: Vec<SourceReport>,
    /// Things the user should know.
    pub warnings: Vec<String>,
}

/// One raster with what is needed to draw it.
#[derive(Debug)]
struct Layer {
    raster: ImageryRaster,
    /// Gain per cell; `None` is 1 everywhere.
    gain: Option<Vec<[f32; 3]>>,
    /// Opacity per cell; `None` is 1 everywhere.
    alpha: Option<Vec<[f32; 1]>>,
}

/// The merged picture of the map: rasters in drawing order with their corrections.
#[derive(Debug)]
pub struct Composite {
    cells: usize,
    cell_m: f64,
    layers: Vec<Layer>,
}

/// What `build_layers` and `render_topdown` take: the composite and the project it was made for.
#[derive(Debug, Clone, Copy)]
pub struct TileImagery<'a> {
    /// The merged imagery.
    pub composite: &'a Composite,
    /// The project it was made for.
    pub project: &'a Project,
}

type Means = Vec<Option<[f32; 3]>>;

/// Merges `rasters` (in the order of `sources.imagery`) into one picture of the map.
///
/// # Errors
///
/// [`ImageryError::Projection`] if a position of the map cannot be converted.
pub fn merge(
    rasters: Vec<ImageryRaster>,
    project: &Project,
) -> Result<(Composite, MergeReport), ImageryError> {
    let cells = usize::try_from(project.grid.land_size)
        .map_err(|_| ImageryError::Unsupported("the land grid is too large".to_string()))?;
    let cell_m = project.map_size_m() / f64::from(project.grid.land_size);
    let target_m = real_px_m(project);
    let means = coarse_means(&rasters, project, cells, cell_m)?;
    let total = cells * cells;

    let mut sources: Vec<SourceReport> = rasters
        .iter()
        .zip(&means)
        .map(|(r, m)| SourceReport {
            label: r.label().to_string(),
            coverage: share(m.iter().flatten().count(), total),
            overlap: 0.0,
            median_gain: [1.0; 3],
            changed_share: 0.0,
        })
        .collect();
    let mut warnings = Vec::new();
    let mut gains: Vec<Option<Vec<[f32; 3]>>> = vec![None; rasters.len()];

    // the reference: most cells, the first listed on a tie
    let reference = (0..rasters.len()).min_by_key(|&i| {
        (
            std::cmp::Reverse(means.get(i).map_or(0, |m| m.iter().flatten().count())),
            i,
        )
    });
    if let (Some(reference), true) = (reference, rasters.len() > 1) {
        let mut composite: Means = means.get(reference).cloned().unwrap_or_default();
        let mut todo: Vec<usize> = (0..rasters.len()).filter(|&i| i != reference).collect();
        while !todo.is_empty() {
            // the raster that overlaps the picture so far the most; the first listed on a tie
            let Some((pos, &next)) = todo.iter().enumerate().max_by_key(|&(_, &i)| {
                (
                    means.get(i).map_or(0, |m| overlap_cells(&composite, m)),
                    std::cmp::Reverse(i),
                )
            }) else {
                break;
            };
            todo.remove(pos);
            let Some(own) = means.get(next) else { continue };
            let matching = match_colours(&composite, own, cells);
            if let Some(report) = sources.get_mut(next) {
                report.overlap = share(matching.overlap, own.iter().flatten().count());
                report.median_gain = matching.global;
                report.changed_share = matching.changed_share;
            }
            if !matching.matched {
                let label = rasters.get(next).map_or("", ImageryRaster::label);
                warnings.push(format!(
                    "{label}: overlaps no other image, its colours are not matched"
                ));
            }
            for (slot, (own_mean, gain)) in composite.iter_mut().zip(own.iter().zip(&matching.gain))
            {
                if slot.is_none() {
                    *slot = own_mean.map(|m| apply(m, *gain));
                }
            }
            if let Some(slot) = gains.get_mut(next) {
                *slot = Some(matching.gain);
            }
        }
    }

    // drawing order: the coarsest first, equals with the first listed last (on top)
    let mut order: Vec<usize> = (0..rasters.len()).collect();
    order.sort_by_key(|&i| {
        let effective = rasters
            .get(i)
            .map_or(target_m, |r| r.pixel_m().max(target_m));
        (
            std::cmp::Reverse(sharpness_class(effective)),
            std::cmp::Reverse(i),
        )
    });
    let multiple = rasters.len() > 1;
    let mut slots: Vec<Option<ImageryRaster>> = rasters.into_iter().map(Some).collect();
    let layers = order
        .into_iter()
        .filter_map(|i| {
            let raster = slots.get_mut(i)?.take()?;
            let has_mean: Vec<bool> = means.get(i)?.iter().map(Option::is_some).collect();
            Some(Layer {
                raster,
                gain: gains.get_mut(i).and_then(Option::take),
                alpha: multiple.then(|| feather(&has_mean, cells)),
            })
        })
        .collect();

    Ok((
        Composite {
            cells,
            cell_m,
            layers,
        },
        MergeReport {
            reference,
            sources,
            warnings,
        },
    ))
}

fn share(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        #[expect(clippy::cast_precision_loss, reason = "cell counts are far below 2^52")]
        {
            part as f64 / whole as f64
        }
    }
}

/// Sizes that differ by less than [`SAME_SHARPNESS`] get the same class.
fn sharpness_class(effective_m: f64) -> i64 {
    let class = (effective_m.max(1e-9).ln() / SAME_SHARPNESS.ln()).round();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the logarithm of a pixel size is small"
    )]
    {
        class as i64
    }
}

fn apply(c: [f32; 3], gain: [f32; 3]) -> [f32; 3] {
    [c[0] * gain[0], c[1] * gain[1], c[2] * gain[2]]
}

/// The mean colour per cell of every raster (row 0 at the south), `None` where any sample has no data.
fn coarse_means(
    rasters: &[ImageryRaster],
    project: &Project,
    cells: usize,
    cell_m: f64,
) -> Result<Vec<Means>, ImageryError> {
    let rows: Vec<Result<Vec<Means>, ImageryError>> = (0..cells)
        .into_par_iter()
        .map(|row| {
            let geo = MapGeo::new(project)?;
            let samplers: Vec<RasterSampler<'_>> = rasters
                .iter()
                .map(ImageryRaster::sampler)
                .collect::<Result<_, _>>()?;
            let mut out: Vec<Means> = vec![Vec::with_capacity(cells); rasters.len()];
            for col in 0..cells {
                let mut sums = vec![([0.0_f32; 3], true); rasters.len()];
                for j in 0..CELL_SAMPLES {
                    for i in 0..CELL_SAMPLES {
                        #[expect(clippy::cast_precision_loss, reason = "cell indices are small")]
                        let (x, y) = (
                            (col as f64 + (f64::from(i) + 0.5) / f64::from(CELL_SAMPLES)) * cell_m,
                            (row as f64 + (f64::from(j) + 0.5) / f64::from(CELL_SAMPLES)) * cell_m,
                        );
                        let (lat, lon) = geo.to_geo(x, y)?;
                        for (sampler, (sum, valid)) in samplers.iter().zip(&mut sums) {
                            match sampler.rgb_at(lat, lon) {
                                Some(c) if *valid => {
                                    for (s, v) in sum.iter_mut().zip(c) {
                                        *s += v;
                                    }
                                }
                                _ => *valid = false,
                            }
                        }
                    }
                }
                #[expect(clippy::cast_precision_loss, reason = "16 samples")]
                let count = (CELL_SAMPLES * CELL_SAMPLES) as f32;
                for (mean, (sum, valid)) in out.iter_mut().zip(sums) {
                    mean.push(valid.then(|| [sum[0] / count, sum[1] / count, sum[2] / count]));
                }
            }
            Ok(out)
        })
        .collect();
    let mut means: Vec<Means> = vec![Vec::with_capacity(cells * cells); rasters.len()];
    for row in rows {
        for (all, part) in means.iter_mut().zip(row?) {
            all.extend(part);
        }
    }
    Ok(means)
}

fn overlap_cells(composite: &Means, own: &Means) -> usize {
    composite
        .iter()
        .zip(own)
        .filter(|(a, b)| a.is_some() && b.is_some())
        .count()
}

/// The colour correction of one raster against the picture so far.
struct Matching {
    gain: Vec<[f32; 3]>,
    global: [f32; 3],
    overlap: usize,
    changed_share: f64,
    matched: bool,
}

fn median(values: &mut [f32]) -> f32 {
    values.sort_by(f32::total_cmp);
    let mid = values.len() / 2;
    match (values.get(mid.wrapping_sub(1)), values.get(mid)) {
        (Some(low), Some(high)) if values.len().is_multiple_of(2) => f32::midpoint(*low, *high),
        (_, Some(v)) => *v,
        _ => 1.0,
    }
}

fn match_colours(composite: &Means, own: &Means, cells: usize) -> Matching {
    let overlap = overlap_cells(composite, own);
    let identity = Matching {
        gain: vec![[1.0; 3]; cells * cells],
        global: [1.0; 3],
        overlap,
        changed_share: 0.0,
        matched: false,
    };
    if overlap < MIN_OVERLAP_CELLS {
        return identity;
    }
    let pairs: Vec<([f32; 3], [f32; 3])> = composite
        .iter()
        .zip(own)
        .map(|(a, b)| match (a, b) {
            (Some(a), Some(b)) => (*a, *b),
            _ => ([0.0; 3], [0.0; 3]),
        })
        .collect();
    // One gain for all three channels: the colours of the sharper image are kept and only its brightness is
    // matched. A gain per channel measured on the typical ground (forest) tints the rest (rock, snow, water)
    // wherever the two images relate differently by kind of ground, which satellite scenes and photographs do.
    let usable = |(a, b): &([f32; 3], [f32; 3])| a.iter().chain(b.iter()).all(|&v| v > DARK_FLOOR);
    // the brightness ratio of a cell: the geometric mean of its three channel ratios
    let ratio = |(a, b): &([f32; 3], [f32; 3])| {
        let one = |c: usize| a.get(c).copied().unwrap_or(1.0) / b.get(c).copied().unwrap_or(1.0);
        (one(0) * one(1) * one(2)).cbrt()
    };
    let mut ratios: Vec<f32> = pairs
        .iter()
        .zip(composite.iter().zip(own))
        .filter(|(p, (a, b))| a.is_some() && b.is_some() && usable(p))
        .map(|(p, _)| ratio(p))
        .collect();
    let global = if ratios.len() >= MIN_OVERLAP_CELLS {
        median(&mut ratios).clamp(GAIN_LIMITS.0, GAIN_LIMITS.1)
    } else {
        1.0
    };
    let mut plane = vec![global; cells * cells];
    let (mut changed, mut measured) = (0_usize, 0_usize);
    for (index, slot) in plane.iter_mut().enumerate() {
        let (Some(Some(_)), Some(Some(_)), Some(pair)) =
            (composite.get(index), own.get(index), pairs.get(index))
        else {
            continue;
        };
        if !usable(pair) {
            continue;
        }
        measured += 1;
        let local = ratio(pair);
        let factor = local / global;
        if (1.0 / (1.0 + CONTENT_CHANGE)..=1.0 + CONTENT_CHANGE).contains(&factor) {
            *slot = local;
        } else {
            changed += 1;
        }
    }
    for _ in 0..BLUR_PASSES {
        box_blur(&mut plane, cells, BLUR_RADIUS);
    }
    let gain: Vec<[f32; 3]> = plane
        .iter()
        .map(|&v| [v.clamp(GAIN_LIMITS.0, GAIN_LIMITS.1); 3])
        .collect();
    Matching {
        gain,
        global: [global; 3],
        overlap,
        changed_share: share(changed, measured),
        matched: true,
    }
}

/// One pass of a box blur of the given radius over a square grid, the edge repeated.
fn box_blur(grid: &mut [f32], n: usize, radius: usize) {
    if n == 0 {
        return;
    }
    let window = radius * 2 + 1;
    #[expect(clippy::cast_precision_loss, reason = "a window of a few cells")]
    let width = window as f32;
    let at = |line: &[f32], i: isize| -> f32 {
        let clamped = i.clamp(0, isize::try_from(n - 1).unwrap_or(0));
        usize::try_from(clamped)
            .ok()
            .and_then(|k| line.get(k))
            .copied()
            .unwrap_or(0.0)
    };
    let r = isize::try_from(radius).unwrap_or(0);
    let mut line = vec![0.0_f32; n];
    // rows, then columns
    for pass in 0..2 {
        for k in 0..n {
            for (i, slot) in line.iter_mut().enumerate() {
                let idx = if pass == 0 { k * n + i } else { i * n + k };
                *slot = grid.get(idx).copied().unwrap_or(0.0);
            }
            for i in 0..n {
                let centre = isize::try_from(i).unwrap_or(0);
                let sum: f32 = (-r..=r).map(|d| at(&line, centre + d)).sum();
                let idx = if pass == 0 { k * n + i } else { i * n + k };
                if let Some(slot) = grid.get_mut(idx) {
                    *slot = sum / width;
                }
            }
        }
    }
}

/// Opacity per cell: 0 in cells without a mean colour, rising to 1 over [`FEATHER_CELLS`] cells. The edge
/// of the map does not count as an edge.
fn feather(has_mean: &[bool], n: usize) -> Vec<[f32; 1]> {
    const FAR: u32 = u32::MAX / 4;
    let mut dist: Vec<u32> = has_mean.iter().map(|&m| if m { FAR } else { 0 }).collect();
    let get = |d: &[u32], x: isize, y: isize| -> Option<u32> {
        let n_i = isize::try_from(n).ok()?;
        if x < 0 || y < 0 || x >= n_i || y >= n_i {
            return None;
        }
        d.get(usize::try_from(y * n_i + x).ok()?).copied()
    };
    // chamfer distance with weights 3 (straight) and 4 (diagonal)
    for pass in 0..2 {
        let order: Vec<(isize, isize)> = {
            let n_i = isize::try_from(n).unwrap_or(0);
            let mut v = Vec::with_capacity(n * n);
            for y in 0..n_i {
                for x in 0..n_i {
                    v.push(if pass == 0 {
                        (x, y)
                    } else {
                        (n_i - 1 - x, n_i - 1 - y)
                    });
                }
            }
            v
        };
        let neighbours: [(isize, isize, u32); 4] = if pass == 0 {
            [(-1, 0, 3), (0, -1, 3), (-1, -1, 4), (1, -1, 4)]
        } else {
            [(1, 0, 3), (0, 1, 3), (1, 1, 4), (-1, 1, 4)]
        };
        for (x, y) in order {
            let mut best = get(&dist, x, y).unwrap_or(0);
            for (dx, dy, w) in neighbours {
                if let Some(d) = get(&dist, x + dx, y + dy) {
                    best = best.min(d.saturating_add(w));
                }
            }
            let n_i = isize::try_from(n).unwrap_or(0);
            if let Some(cell) = usize::try_from(y * n_i + x)
                .ok()
                .and_then(|idx| dist.get_mut(idx))
            {
                *cell = best;
            }
        }
    }
    dist.iter()
        .map(|&d| {
            if d >= FAR {
                [1.0]
            } else {
                #[expect(clippy::cast_precision_loss, reason = "distances are small")]
                let cells_away = d as f32 / 3.0;
                [(cells_away / FEATHER_CELLS).min(1.0)]
            }
        })
        .collect()
}

/// Bilinear value of a per-cell field at the map position (`x`, `y`) in metres; cell centres carry the
/// values and the edge is repeated.
fn field_at<const C: usize>(
    field: &[[f32; C]],
    cells: usize,
    cell_m: f64,
    (x, y): (f64, f64),
) -> [f32; C] {
    let last = cells.saturating_sub(1);
    let clamp = |v: f64| -> (usize, f32) {
        let pos = (v / cell_m - 0.5).clamp(0.0, f64::from(u32::try_from(last).unwrap_or(0)));
        let low = pos.floor();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 0..=last above"
        )]
        let (low_i, frac) = (low as usize, (pos - low) as f32);
        (low_i.min(last), frac)
    };
    let ((cx, wx), (cy, wy)) = (clamp(x), clamp(y));
    let (cx1, cy1) = ((cx + 1).min(last), (cy + 1).min(last));
    let at = |cx: usize, cy: usize| field.get(cy * cells + cx).copied().unwrap_or([1.0; C]);
    let [sw, se, nw, ne] = [at(cx, cy), at(cx1, cy), at(cx, cy1), at(cx1, cy1)];
    let mut out = [0.0_f32; C];
    for (i, slot) in out.iter_mut().enumerate() {
        let get = |v: &[f32; C]| v.get(i).copied().unwrap_or(1.0);
        let low = get(&sw) * (1.0 - wx) + get(&se) * wx;
        let high = get(&nw) * (1.0 - wx) + get(&ne) * wx;
        *slot = low * (1.0 - wy) + high * wy;
    }
    out
}

impl Composite {
    /// The colours of a rectangle of the map: pixel centres, row 0 at the north. `None` where no image
    /// covers the pixel. One call per tile or render; calls can run in parallel.
    ///
    /// # Errors
    ///
    /// [`ImageryError::Projection`] if a position cannot be converted, [`ImageryError::Unsupported`] for a
    /// rectangle of more than 2^28 pixels.
    pub fn paint(
        &self,
        project: &Project,
        rect: &MapRect,
    ) -> Result<Vec<Option<[u8; 3]>>, ImageryError> {
        let pixels = u64::from(rect.width) * u64::from(rect.height);
        if pixels > MAX_PAINT_PIXELS {
            return Err(ImageryError::Unsupported(format!(
                "{pixels} pixels in one picture are too many"
            )));
        }
        let geo = MapGeo::new(project)?;
        let samplers: Vec<RasterSampler<'_>> = self
            .layers
            .iter()
            .map(|l| l.raster.sampler())
            .collect::<Result<_, _>>()?;
        let mut out = Vec::with_capacity(usize::try_from(pixels).unwrap_or(0));
        for row in 0..rect.height {
            let y = rect.north - (f64::from(row) + 0.5) * rect.step_y;
            for col in 0..rect.width {
                let x = rect.west + (f64::from(col) + 0.5) * rect.step_x;
                let (lat, lon) = geo.to_geo(x, y)?;
                let mut colour: Option<[f32; 3]> = None;
                for (layer, sampler) in self.layers.iter().zip(&samplers) {
                    let Some(own) = sampler.rgb_at(lat, lon) else {
                        continue;
                    };
                    let own = match &layer.gain {
                        Some(g) => apply(
                            own,
                            field_at(g, self.cells, self.cell_m, (x, y))[..3]
                                .try_into()
                                .unwrap_or([1.0; 3]),
                        ),
                        None => own,
                    };
                    colour = Some(match colour {
                        None => own,
                        Some(below) => {
                            let alpha = layer
                                .alpha
                                .as_ref()
                                .map_or(1.0, |a| field_at(a, self.cells, self.cell_m, (x, y))[0]);
                            [
                                below[0] + (own[0] - below[0]) * alpha,
                                below[1] + (own[1] - below[1]) * alpha,
                                below[2] + (own[2] - below[2]) * alpha,
                            ]
                        }
                    });
                }
                out.push(colour.map(to_bytes));
            }
        }
        Ok(out)
    }
}

fn to_bytes(c: [f32; 3]) -> [u8; 3] {
    let byte = |v: f32| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "rounded and clamped to 0..=255"
        )]
        {
            v.round().clamp(0.0, 255.0) as u8
        }
    };
    [byte(c[0]), byte(c[1]), byte(c[2])]
}
