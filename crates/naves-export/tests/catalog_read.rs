// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading the road catalog from the game's archives, against a fake `dayzmake` that lists and takes files out
//! of trees of synthetic models (see `naves_export::testing`).

#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use std::path::{Path, PathBuf};

use naves_core::catalog::testing::synthetic_catalog;
use naves_export::catalog_read::{CatalogReadError, road_catalog, unpack_roads, unpack_used};
use naves_export::testing::FakeEnv;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    env: FakeEnv,
    trees: PathBuf,
    cache: PathBuf,
    project: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let env = FakeEnv::new(&root).unwrap();
    let trees = root.join("pbo-trees");
    // things the catalog must not stumble over, nor take out of the archives
    std::fs::write(
        trees.join("structures_roads/Parts/runway_main.p3d"),
        b"not a model",
    )
    .unwrap();
    std::fs::create_dir_all(trees.join("structures_roads/Decals")).unwrap();
    std::fs::write(
        trees.join("structures_roads/Decals/Decal_Manhole.p3d"),
        b"not a model",
    )
    .unwrap();
    std::fs::write(
        trees.join("structures_roads/Panels/Panel_Concrete_Dam.p3d"),
        b"not a model",
    )
    .unwrap();
    std::fs::write(
        trees.join("structures_sakhal/roads/bridges/Bridge_Wood2_Snow.p3d"),
        b"not a model",
    )
    .unwrap();
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    Fixture {
        cache: root.join("cache"),
        _dir: dir,
        root,
        env,
        trees,
        project,
    }
}

/// The dayzmake calls so far, one line each.
fn calls(f: &Fixture) -> Vec<String> {
    std::fs::read_to_string(f.root.join("dayzmake-calls.log"))
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// How often an archive was listed.
fn runs(f: &Fixture) -> usize {
    calls(f)
        .iter()
        .filter(|c| c.starts_with("inspect") && c.contains("structures_roads"))
        .count()
}

#[test]
fn the_catalog_is_read_from_the_archives() {
    let f = fixture();
    let catalog = road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert_eq!(catalog, synthetic_catalog());
    assert_eq!(catalog.warnings().len(), 0, "{:?}", catalog.warnings());
    assert_eq!(catalog.panels().len(), 2);
    assert!(catalog.pipe().is_some());
    assert!(
        catalog
            .bridges()
            .iter()
            .any(|b| b.model == "dz\\structures_sakhal\\roads\\bridges\\bridge_wood2.p3d")
    );
    assert_eq!(runs(&f), 1);
}

#[test]
fn only_what_the_catalog_uses_is_taken_out() {
    let f = fixture();
    road_catalog(&f.env.tools(), &f.cache).unwrap();
    let taken: Vec<String> = calls(&f)
        .into_iter()
        .filter(|c| c.starts_with("cat"))
        .collect();
    assert_ne!(taken.len(), 0);
    for unwanted in [
        "runway_main",
        "Decal_Manhole",
        "Panel_Concrete_Dam",
        "Wood2_Snow",
    ] {
        assert!(taken.iter().all(|c| !c.contains(unwanted)), "{unwanted}");
    }
    assert!(taken.iter().any(|c| c.contains("Misc_Concretepipe")));
    // no archive is unpacked whole for the catalog
    assert!(calls(&f).iter().all(|c| !c.starts_with("unpack")));
}

#[test]
fn the_model_paths_follow_the_prefix_the_archive_names() {
    let f = fixture();
    std::fs::write(
        f.trees.join("structures_industrial/.prefix"),
        "DZ\\structures\\Industrial2",
    )
    .unwrap();
    let catalog = road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert_eq!(
        catalog.pipe().unwrap().model,
        "dz\\structures\\industrial2\\misc\\misc_concretepipe.p3d"
    );
}

#[test]
fn the_folders_are_found_in_any_case() {
    let f = fixture();
    let server = &f.env.server;
    std::fs::rename(server.join("addons"), server.join("Addons")).unwrap();
    std::fs::rename(server.join("sakhal/addons"), server.join("sakhal/Addons")).unwrap();
    let catalog = road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert_eq!(catalog, synthetic_catalog());
}

#[test]
fn without_the_sakhal_archive_the_catalog_warns_and_has_no_wood2() {
    let f = fixture();
    let sakhal = f.env.server.join("sakhal/addons/structures_sakhal.pbo");
    std::fs::remove_file(&sakhal).unwrap();
    let catalog = road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert!(
        !catalog
            .bridges()
            .iter()
            .any(|b| b.model.contains("bridge_wood2"))
    );
    assert!(
        catalog
            .warnings()
            .iter()
            .any(|w| w.contains("structures_sakhal.pbo")),
        "{:?}",
        catalog.warnings()
    );
    // the archive appears: the cache is made again
    std::fs::write(&sakhal, b"fake sakhal pbo").unwrap();
    let catalog = road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert!(
        catalog
            .bridges()
            .iter()
            .any(|b| b.model.contains("bridge_wood2"))
    );
    assert_eq!(runs(&f), 2);
}

#[test]
fn an_optional_archive_that_cannot_be_listed_is_a_warning() {
    let f = fixture();
    std::fs::remove_dir_all(f.trees.join("structures_industrial")).unwrap();
    let catalog = road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert!(catalog.pipe().is_none());
    assert!(
        catalog
            .warnings()
            .iter()
            .any(|w| w.contains("structures_industrial.pbo") && w.contains("cannot read")),
        "{:?}",
        catalog.warnings()
    );
}

#[test]
fn the_catalog_is_cached() {
    let f = fixture();
    let first = road_catalog(&f.env.tools(), &f.cache).unwrap();
    let second = road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert_eq!(first, second);
    assert_eq!(runs(&f), 1, "the second call must not run dayzmake");
    assert_eq!(
        calls(&f)
            .iter()
            .filter(|c| c.starts_with("inspect"))
            .count(),
        3
    );
}

#[test]
fn a_changed_archive_rebuilds_the_cache() {
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
    // only the modification time of another archive changes
    let pbo = std::fs::File::options()
        .write(true)
        .open(f.env.server.join("addons/structures_industrial.pbo"))
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
fn a_missing_road_archive_names_the_file() {
    let f = fixture();
    std::fs::remove_file(f.env.server.join("addons/structures_roads.pbo")).unwrap();
    let e = road_catalog(&f.env.tools(), &f.cache).unwrap_err();
    assert!(matches!(e, CatalogReadError::MissingPbo(_)), "{e}");
    assert!(e.to_string().contains("structures_roads.pbo"), "{e}");
    assert_eq!(calls(&f).len(), 0);
}

#[test]
fn a_damaged_road_piece_names_the_model() {
    let f = fixture();
    std::fs::write(
        f.trees.join("structures_roads/Parts/asf3_25.p3d"),
        b"ODOL damaged",
    )
    .unwrap();
    let e = road_catalog(&f.env.tools(), &f.cache).unwrap_err();
    assert!(matches!(e, CatalogReadError::Model { .. }), "{e}");
    assert!(e.to_string().contains("asf3_25"), "{e}");
}

#[test]
fn a_damaged_bridge_part_is_left_out_with_a_warning() {
    let f = fixture();
    std::fs::write(
        f.trees.join("structures_roads/Bridges/Bridge_Stone_25.p3d"),
        b"ODOL damaged",
    )
    .unwrap();
    let catalog = road_catalog(&f.env.tools(), &f.cache).unwrap();
    assert!(
        !catalog
            .bridges()
            .iter()
            .any(|b| b.model.contains("bridge_stone_25"))
    );
    assert!(
        catalog
            .warnings()
            .iter()
            .any(|w| w.contains("bridge_stone_25") && w.contains("cannot be read")),
        "{:?}",
        catalog.warnings()
    );
}

#[test]
fn a_failing_dayzmake_is_an_error_with_its_output() {
    let f = fixture();
    f.env
        .replace_dayzmake("echo 'cannot read the pbo' >&2; exit 3")
        .unwrap();
    let e = road_catalog(&f.env.tools(), &f.cache).unwrap_err();
    assert!(matches!(e, CatalogReadError::Dayzmake { .. }), "{e}");
    assert!(e.to_string().contains("cannot read the pbo"), "{e}");
}

#[test]
fn an_archive_without_a_prefix_is_an_error() {
    let f = fixture();
    f.env
        .replace_dayzmake("echo 'Header extensions:'; echo '====='")
        .unwrap();
    let e = road_catalog(&f.env.tools(), &f.cache).unwrap_err();
    assert!(matches!(e, CatalogReadError::NoPrefix(_)), "{e}");
}

#[test]
fn extraction_happens_outside_the_project() {
    let f = fixture();
    road_catalog(&f.env.tools(), &f.cache).unwrap();
    let dests: Vec<String> = calls(&f)
        .iter()
        .filter(|c| c.starts_with("cat"))
        .filter_map(|c| c.rsplit(' ').next().map(str::to_string))
        .collect();
    assert_ne!(dests.len(), 0);
    for dest in dests {
        let dest = Path::new(&dest);
        assert!(!dest.starts_with(&f.project), "{}", dest.display());
        assert!(!dest.starts_with(&f.cache), "{}", dest.display());
        assert!(!dest.exists(), "the extraction folder must be removed");
    }
    // the project folder is untouched, and the cache holds nothing but the parsed result
    assert_eq!(walk(&f.project).len(), 0);
    for file in walk(&f.cache) {
        assert!(
            file.extension().is_some_and(|e| e == "json"),
            "{}",
            file.display()
        );
    }
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
    assert_ne!(catalog.bridges().len(), 0);
    assert_eq!(catalog.panels().len(), 2);
    assert!(catalog.pipe().is_some());
    assert!(
        catalog
            .bridges()
            .iter()
            .any(|b| b.model.contains("bridge_wood2"))
    );
    assert!(
        catalog
            .bridges()
            .iter()
            .any(|b| b.model.contains("bridge_concrete3"))
    );
}

#[test]
fn unpack_roads_puts_the_models_where_the_world_refers_to_them() {
    let f = fixture();
    let dest = f.root.join("p");
    let pbo = f.env.server.join("addons/structures_roads.pbo");
    unpack_roads(&f.env.tools(), &pbo, &dest).unwrap();
    assert!(dest.join("dz/structures/roads/Parts/asf3_25.p3d").is_file());
    assert!(
        dest.join("dz/structures/roads/Bridges/Bridge_Metal_25.p3d")
            .is_file()
    );
    let e = unpack_roads(&f.env.tools(), &f.root.join("nope.pbo"), &dest).unwrap_err();
    assert!(matches!(e, CatalogReadError::MissingPbo(_)), "{e}");
}

#[test]
fn unpack_used_provides_the_archives_of_the_models_a_world_uses() {
    let f = fixture();
    let dest = f.root.join("p");
    let wood2 = "dz\\structures_sakhal\\roads\\bridges\\bridge_wood2.p3d";
    let road = "dz\\structures\\roads\\parts\\asf3_25.p3d";
    // only road models: nothing more is unpacked
    unpack_used(&f.env.tools(), [road], &dest).unwrap();
    assert!(!dest.join("dz/structures_sakhal").exists());
    assert!(!dest.join("dz/structures/industrial").exists());
    // a Sakhal bridge: its archive is below the P drive
    unpack_used(&f.env.tools(), [road, wood2], &dest).unwrap();
    assert!(dest.join("dz/structures_sakhal/roads/bridges").is_dir());
    assert!(!dest.join("dz/structures/industrial").exists());
    // an industrial model
    let pipe = "dz\\structures\\industrial\\misc\\misc_concretepipe.p3d";
    unpack_used(&f.env.tools(), [pipe], &dest).unwrap();
    assert!(
        dest.join("dz/structures/industrial")
            .read_dir()
            .unwrap()
            .next()
            .is_some()
    );
}

#[test]
fn a_missing_archive_names_the_model_that_needs_it() {
    let f = fixture();
    std::fs::remove_file(f.env.server.join("sakhal/addons/structures_sakhal.pbo")).unwrap();
    let wood2 = "dz\\structures_sakhal\\roads\\bridges\\bridge_wood2.p3d";
    let e = unpack_used(&f.env.tools(), [wood2], &f.root.join("p")).unwrap_err();
    let CatalogReadError::MissingArchive { model, pbo } = &e else {
        panic!("{e}");
    };
    assert_eq!(model, wood2);
    assert!(pbo.ends_with("structures_sakhal.pbo"), "{pbo:?}");
    assert!(e.to_string().contains("bridge_wood2"), "{e}");
}

/// Writes fuzz seeds of bridge models with faces from the synthetic trees:
/// `cargo test -p naves-export --test catalog_read -- --ignored write_fuzz_seeds`.
#[test]
#[ignore = "regenerates files under fuzz/corpus/odol_read"]
fn write_fuzz_seeds() {
    let dir = tempfile::tempdir().unwrap();
    naves_export::testing::write_pbo_trees(dir.path()).unwrap();
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/odol_read");
    for (seed, model) in [
        (
            "bridge_blocks",
            "structures_roads/Bridges/Bridge_Metal_25.p3d",
        ),
        (
            "bridge_arch",
            "structures_roads/Bridges/Bridge_Stone_25.p3d",
        ),
        ("panel", "structures_roads/Panels/Panel_Concrete_1.p3d"),
    ] {
        std::fs::copy(dir.path().join(model), corpus.join(seed)).unwrap();
    }
}
