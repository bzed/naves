// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

// One case of the `road_line` fuzz target, shared (with `include!`) by the target and by the seed test in
// `crates/naves-core/tests/fuzz_seeds.rs`, so that the two cannot drift apart.
//
// The input is JSON: `{"points": [[x, y], ...], "tags": {"key": "value", ...}}`.

mod road_line_case {
    use std::collections::BTreeMap;

    use naves_core::edits::{By, NodeIndex, WayEdits, check_line};
    use naves_core::heightmap::MapGeo;
    use naves_core::osm::OsmWays;
    use naves_core::project::Project;

    type Tags = BTreeMap<String, String>;

    fn parse(data: &[u8]) -> Option<(Vec<(f64, f64)>, Tags)> {
        let value: serde_json::Value = serde_json::from_slice(data).ok()?;
        let points = value
            .get("points")?
            .as_array()?
            .iter()
            .map(|p| {
                let p = p.as_array()?;
                Some((p.first()?.as_f64()?, p.get(1)?.as_f64()?))
            })
            .collect::<Option<Vec<_>>>()?;
        let tags = value
            .get("tags")?
            .as_object()?
            .iter()
            .map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect::<Option<BTreeMap<_, _>>>()?;
        Some((points, tags))
    }

    /// `None` if the input is not a case; otherwise whether `check_line` and `add_line` both accepted it.
    pub fn run(data: &[u8]) -> Option<bool> {
        let (points, tags) = parse(data)?;
        let project = Project::new("Fuzz", 47.0, 12.0, 4000.0);
        let frame = MapGeo::new(&project).ok()?;
        let index = NodeIndex::new(&OsmWays::default(), &frame);
        let checked = check_line("line", &points, frame.map_size_m());
        let mut edits = WayEdits::default();
        let added = edits.add_line(&index, &frame, tags, &points, By::Ai);
        Some(checked.is_ok() && added.is_ok())
    }
}
