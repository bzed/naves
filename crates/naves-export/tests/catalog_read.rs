// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading the road catalog from the game's `structures_roads.pbo`, against a fake `armake2` that
//! "unpacks" a tree of synthetic models.

#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::path::{Path, PathBuf};

use naves_core::catalog::testing::{synthetic_catalog, synthetic_models};
use naves_export::catalog_read::{CatalogReadError, road_catalog};
use naves_export::testing::FakeEnv;
use naves_formats::odol::synthetic::model_bytes;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    env: FakeEnv,
    tree: PathBuf,
    calls: PathBuf,
    dests: PathBuf,
    cache: PathBuf,
    project: PathBuf,
}

/// `asf3_10 25` stays, `bridge_stone_25` becomes `Bridge_Stone_25`: the game's file names are mixed case.
fn game_case(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("bridge_") {
        let parts: Vec<String> = rest
            .split('_')
            .map(|p| {
                let mut c = p.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            })
            .collect();
        return format!("Bridge_{}", parts.join("_"));
    }
    name.to_string()
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let env = FakeEnv::new(&root).unwrap();
    let tree = root.join("pbo-content");
    for (path, model) in synthetic_models() {
        let (folder, file) = if path.contains("\\bridges\\") {
            ("Bridges", path.rsplit('\\').next().unwrap())
        } else {
            ("Parts", path.rsplit('\\').next().unwrap())
        };
        let stem = game_case(file.strip_suffix(".p3d").unwrap());
        let points: Vec<(&str, [f32; 3])> = model
            .memory
            .iter()
            .map(|(k, v)| (k.as_str(), *v.first().unwrap()))
            .collect();
        let target = tree.join(folder).join(format!("{stem}.p3d"));
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, model_bytes(&points)).unwrap();
    }
    // things the catalog must not stumble over
    std::fs::write(tree.join("Parts/runway_main.p3d"), b"not a model").unwrap();
    std::fs::create_dir_all(tree.join("Decals")).unwrap();
    std::fs::write(tree.join("Decals/Decal_Manhole.p3d"), b"not a model").unwrap();

    let calls = root.join("calls.log");
    let dests = root.join("dests.log");
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    env.replace_armake2(&format!(
        r#"case "$1" in
  unpack) echo run >> "{calls}"; echo "$3" >> "{dests}"; cp -r "{tree}"/. "$3"/ ;;
  *) echo "unexpected armake2 call: $*" >&2; exit 2 ;;
esac"#,
        calls = calls.display(),
        dests = dests.display(),
        tree = tree.display(),
    ))
    .unwrap();
    let pbo = env.server.join("addons/structures_roads.pbo");
    std::fs::create_dir_all(pbo.parent().unwrap()).unwrap();
    std::fs::write(&pbo, b"pbo version 1").unwrap();
    Fixture {
        cache: root.join("cache"),
        _dir: dir,
        root,
        env,
        tree,
        calls,
        dests,
        project,
    }
}

fn runs(f: &Fixture) -> usize {
    std::fs::read_to_string(&f.calls).map_or(0, |t| t.lines().count())
}

#[test]
fn the_catalog_is_read_from_the_unpacked_models() {
    let f = fixture();
    let catalog = road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert_eq!(catalog, synthetic_catalog());
    assert_eq!(runs(&f), 1);
}

#[test]
fn the_catalog_is_cached() {
    let f = fixture();
    let first = road_catalog(&f.env.tools(), &f.cache).unwrap();
    let second = road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert_eq!(first, second);
    assert_eq!(runs(&f), 1, "the second call must not run armake2");
}

#[test]
fn a_changed_pbo_rebuilds_the_cache() {
    let f = fixture();
    road_catalog(&f.env.tools(), &f.cache).unwrap();
    // the size changes
    std::fs::write(
        f.env.server.join("addons/structures_roads.pbo"),
        b"pbo version 2, a different size",
    )
    .unwrap();
    road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert_eq!(runs(&f), 2);
    // only the modification time changes
    let pbo = std::fs::File::options()
        .write(true)
        .open(f.env.server.join("addons/structures_roads.pbo"))
        .unwrap();
    pbo.set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000))
        .unwrap();
    road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert_eq!(runs(&f), 3);
}

#[test]
fn a_corrupt_cache_is_rebuilt() {
    let f = fixture();
    road_catalog(&f.env.tools(), &f.cache).unwrap();
    for entry in walk(&f.cache) {
        std::fs::write(entry, b"{ not json").unwrap();
    }
    let again = road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert_eq!(again, synthetic_catalog());
    assert_eq!(runs(&f), 2);
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .map(|e| e.unwrap().into_path())
        .filter(|p| p.is_file())
        .collect()
}

#[test]
fn a_missing_pbo_names_the_file() {
    let f = fixture();
    std::fs::remove_file(f.env.server.join("addons/structures_roads.pbo")).unwrap();
    let e = road_catalog(&f.env.tools(), &f.cache).unwrap_err();
    assert!(matches!(e, CatalogReadError::MissingPbo(_)), "{e}");
    assert!(e.to_string().contains("structures_roads.pbo"), "{e}");
    assert_eq!(runs(&f), 0);
}

#[test]
fn a_damaged_model_names_the_model() {
    let f = fixture();
    std::fs::write(f.tree.join("Parts/asf3_25.p3d"), b"ODOL damaged").unwrap();
    let e = road_catalog(&f.env.tools(), &f.cache).unwrap_err();
    assert!(matches!(e, CatalogReadError::Model { .. }), "{e}");
    assert!(e.to_string().contains("asf3_25"), "{e}");
}

#[test]
fn a_failing_armake2_is_an_error_with_its_output() {
    let f = fixture();
    f.env
        .replace_armake2("echo 'cannot read the pbo' >&2; exit 3")
        .unwrap();
    let e = road_catalog(&f.env.tools(), &f.cache).unwrap_err();
    assert!(matches!(e, CatalogReadError::Armake2 { .. }), "{e}");
    assert!(e.to_string().contains("cannot read the pbo"), "{e}");
}

#[test]
fn extraction_happens_outside_the_project() {
    let f = fixture();
    road_catalog(&f.env.tools(), &f.cache).unwrap();
    let dests = std::fs::read_to_string(&f.dests).unwrap();
    let dest = Path::new(dests.trim());
    assert!(!dest.starts_with(&f.project), "{}", dest.display());
    assert!(!dest.starts_with(&f.cache), "{}", dest.display());
    assert!(!dest.exists(), "the extraction folder must be removed");
    // the project folder is untouched, and the cache holds nothing but the parsed result
    assert!(walk(&f.project).is_empty());
    for file in walk(&f.cache) {
        assert!(
            file.extension().is_some_and(|e| e == "json"),
            "{}",
            file.display()
        );
    }
    let _ = &f.root;
}

#[test]
#[ignore = "needs DayZ"]
fn catalog_of_the_installed_game() {
    use naves_core::catalog::{Family, PieceKind};
    let tools = naves_export::tools::Tools::locate().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let catalog = road_catalog(&tools, cache.path()).unwrap();
    for family in Family::ALL {
        assert!(
            catalog.pieces(family).len() >= 14,
            "{family:?}: {}",
            catalog.pieces(family).len()
        );
        assert!(
            catalog
                .pieces(family)
                .iter()
                .any(|p| p.kind == PieceKind::Arc)
        );
    }
    let junctions = Family::ALL
        .iter()
        .flat_map(|a| Family::ALL.iter().map(move |b| (*a, *b)))
        .map(|(a, b)| {
            usize::from(catalog.t_junction(a, b).is_some())
                + usize::from(catalog.x_junction(a, b).is_some())
        })
        .sum::<usize>();
    assert!(junctions >= 15, "{junctions} junctions");
    assert!(!catalog.bridges().is_empty());
}
