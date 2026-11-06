// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Renders the map of a project to a PNG: `cargo run -p naves-core --example render_png -- <project dir>
//! <hillshade|slope|height|surface> <width> <out.png>`. Used for the screenshots in the manual.

use std::path::PathBuf;

use naves_core::dem::Dem;
use naves_core::heightmap::build_heightmap;
use naves_core::project::Project;
use naves_core::render::{Overlay, RenderRequest, render_topdown};
use naves_core::surface::project_surfaces;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().ok_or("missing project directory")?);
    let overlay = match args.next().as_deref() {
        Some("slope") => Overlay::Slope,
        Some("height") => Overlay::Height,
        Some("surface") => Overlay::Surface,
        _ => Overlay::Hillshade,
    };
    let px: u32 = args.next().ok_or("missing width")?.parse()?;
    let out = PathBuf::from(args.next().ok_or("missing output file")?);

    let project = Project::load(&dir)?;
    let files: Vec<PathBuf> = project.sources.dem.iter().map(|p| dir.join(p)).collect();
    let hm = build_heightmap(&project, &Dem::open(&files)?)?;
    let size = project.map_size_m();
    let request = RenderRequest {
        x0: 0.0,
        y0: 0.0,
        x1: size,
        y1: size,
        px,
        overlay,
    };
    let surfaces = if overlay == Overlay::Surface {
        Some(project_surfaces(&project, &dir, &hm)?)
    } else {
        None
    };
    render_topdown(&hm, &request, surfaces.as_ref())?.save(out)?;
    Ok(())
}
