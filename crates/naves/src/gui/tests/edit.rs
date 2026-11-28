// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The edit session: edits are saved at once, undone and redone, and a source change keeps them.

use egui_kittest::kittest::Queryable;
use naves_core::edits::{By, ElementId, WayEdits};
use naves_core::osm::OsmWay;

use super::*;
use crate::gui::edit::{EditSession, EditState, Origin};
use crate::gui::worker::RoadsState;

/// A session on the roads of the Königssee fixture.
fn session(root: &Path) -> (EditSession, PathBuf) {
    let dir = with_roads(root);
    let project = Project::load(&dir).unwrap();
    let source = naves_export::generate::read_source(&dir, &project).unwrap();
    (EditSession::new(source), dir)
}

fn first_way(session: &EditSession) -> OsmWay {
    session.source.extract.ways.first().unwrap().clone()
}

fn delete(session: &mut EditSession, dir: &Path, way: &OsmWay) {
    session
        .apply(dir, |edits, _| edits.delete_way(way, By::User))
        .unwrap();
}

#[test]
fn an_edit_is_saved_at_once() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, dir) = session(root.path());
    let way = first_way(&session);
    delete(&mut session, &dir, &way);
    let saved = WayEdits::load(&dir).unwrap();
    assert_eq!(saved.ways.len(), 1);
    assert_eq!(saved, session.source.edits);
    assert!(session.way(way.id).is_none());
    assert!(session.original(way.id).is_some());
    assert_eq!(session.origin(way.id), Origin::EditedOsm);
    assert!(session.dirty);
}

#[test]
fn the_projected_ways_are_built_once_per_change_of_the_edits() {
    use crate::gui::edit::hit::{hit, tolerance_m};
    use crate::gui::edit::overlay::overlay;
    let root = tempfile::tempdir().unwrap();
    let (mut session, dir) = session(root.path());
    assert_eq!(
        session.projection_builds(),
        1,
        "built when the session starts"
    );
    let way = first_way(&session);
    let at = session.projected()[0].points[0].1;
    // frames and clicks read the cache
    for _ in 0..5 {
        let _ = overlay(&session, None, &[]);
        let _ = hit(&session, at, tolerance_m(100.0, 1000.0));
    }
    assert_eq!(session.projection_builds(), 1);
    delete(&mut session, &dir, &way);
    assert_eq!(session.projection_builds(), 2, "refreshed after an edit");
    assert!(session.projected().iter().all(|p| p.id != way.id));
    assert_eq!(session.undo(&dir), Ok(true));
    assert_eq!(session.projection_builds(), 3, "and after an undo");
    assert!(session.projected().iter().any(|p| p.id == way.id));
}

#[test]
fn a_moved_node_marks_its_way_as_changed() {
    use crate::gui::edit::fixture::session as fixture_session;
    use crate::gui::edit::overlay::{OverlayShape, overlay};
    let mut s = fixture_session(&[
        (7, &[(500.0, 500.0), (550.0, 500.0), (600.0, 500.0)]),
        (8, &[(500.0, 600.0), (600.0, 600.0)]),
    ]);
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(s.origin(7), Origin::Osm);
    let before = s.source.extract.ways[0].points[1];
    let (lat, lon) = s.source.frame.to_geo(550.0, 520.0).unwrap();
    s.apply(dir.path(), |edits, _| {
        edits.move_node(ElementId::Osm(701), lat, lon, Some(before), By::User)
    })
    .unwrap();
    assert_eq!(s.origin(7), Origin::EditedOsm, "the road is changed");
    assert_eq!(s.origin(8), Origin::Osm);
    let ticks = overlay(&s, None, &[])
        .iter()
        .filter(|shape| matches!(shape, OverlayShape::Tick { .. }))
        .count();
    assert_eq!(ticks, 2, "ticks at both ends of the moved road");
}

#[test]
fn undo_and_redo_restore_the_file() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, dir) = session(root.path());
    let way = first_way(&session);
    delete(&mut session, &dir, &way);
    assert_eq!(session.undo(&dir), Ok(true));
    assert_eq!(WayEdits::load(&dir).unwrap(), WayEdits::default());
    assert!(session.way(way.id).is_some());
    assert_eq!(session.origin(way.id), Origin::Osm);
    assert_eq!(session.redo(&dir), Ok(true));
    assert_eq!(WayEdits::load(&dir).unwrap().ways.len(), 1);
    assert!(session.way(way.id).is_none());
    assert_eq!(session.undo(&dir), Ok(true));
    assert_eq!(session.undo(&dir), Ok(false));
}

#[test]
fn the_undo_stack_holds_one_hundred() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, dir) = session(root.path());
    for i in 0..=EditSession::UNDO_DEPTH {
        let lat = 47.5 + f64::from(u32::try_from(i).unwrap()) * 1e-5;
        session
            .apply(&dir, |edits, _| {
                edits.new_node(lat, 12.9, By::User).map(|_| ())
            })
            .unwrap();
    }
    for _ in 0..EditSession::UNDO_DEPTH {
        assert_eq!(session.undo(&dir), Ok(true));
    }
    assert_eq!(session.undo(&dir), Ok(false));
}

#[test]
fn a_new_edit_clears_redo() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, dir) = session(root.path());
    let way = first_way(&session);
    delete(&mut session, &dir, &way);
    session.undo(&dir).unwrap();
    assert!(session.can_redo());
    session
        .apply(&dir, |edits, _| {
            edits.new_node(47.5, 12.9, By::User).map(|_| ())
        })
        .unwrap();
    assert!(!session.can_redo());
    assert_eq!(session.redo(&dir), Ok(false));
}

#[test]
fn a_drawn_way_is_drawn() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, dir) = session(root.path());
    let mut added = None;
    session
        .apply(&dir, |edits, _| {
            let a = edits.new_node(47.55, 12.97, By::User)?;
            let b = edits.new_node(47.551, 12.97, By::User)?;
            let tags = [("highway".to_string(), "track".to_string())].into();
            added = Some(edits.add_way(tags, vec![a, b], By::User)?);
            Ok(())
        })
        .unwrap();
    let raw = added.and_then(ElementId::raw).unwrap();
    assert_eq!(session.origin(raw), Origin::Drawn);
    assert!(session.way(raw).is_some());
    assert!(session.original(raw).is_some());
}

#[test]
fn an_edit_that_cannot_be_made_changes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, dir) = session(root.path());
    let error = session
        .apply(&dir, |edits, _| {
            edits.new_node(95.0, 12.9, By::User).map(|_| ())
        })
        .unwrap_err();
    assert!(error.starts_with("The edit cannot be made: "), "{error}");
    assert!(!session.can_undo() && !session.dirty);
    assert!(!dir.join("edits/ways.toml").exists());
}

#[cfg(unix)]
#[test]
fn a_failed_save_is_a_problem_until_the_next_edit() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_session(&mut app);
    let way = app.edit.session().unwrap().source.extract.ways[0].clone();
    std::fs::create_dir_all(dir.join("edits")).unwrap();
    std::fs::set_permissions(dir.join("edits"), std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = app.edit_way(|edits, _| edits.delete_way(&way, By::User));
    std::fs::set_permissions(dir.join("edits"), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err());
    app.refresh_problems();
    assert!(
        app.problems
            .rows
            .iter()
            .any(|p| p.message.starts_with("Could not save the edit: "))
    );
    app.edit_way(|edits, _| edits.delete_way(&way, By::User))
        .unwrap();
    app.refresh_problems();
    assert!(
        app.problems
            .rows
            .iter()
            .all(|p| !p.message.starts_with("Could not save the edit: "))
    );
}

#[test]
fn undo_redo_and_regenerate_are_built() {
    use crate::gui::shortcuts::{Action, action_built};
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let app = app(root.path(), &dir);
    for action in [Action::Undo, Action::Redo, Action::Regenerate] {
        assert!(action_built(action, &app), "{action:?}");
    }
}

#[cfg(unix)]
#[test]
fn a_failed_save_undoes_the_edit() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let (mut session, dir) = session(root.path());
    std::fs::create_dir_all(dir.join("edits")).unwrap();
    std::fs::set_permissions(dir.join("edits"), std::fs::Permissions::from_mode(0o555)).unwrap();
    let before = session.source.edits.clone();
    let way = first_way(&session);
    let result = session.apply(&dir, |edits, _| edits.delete_way(&way, By::User));
    std::fs::set_permissions(dir.join("edits"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let error = result.unwrap_err();
    assert!(error.starts_with("Could not save the edit: "), "{error}");
    assert_eq!(session.source.edits, before);
    assert!(!session.can_undo() && !session.dirty);
    assert!(session.way(way.id).is_some());
}

fn wait_for_the_session(app: &mut NavesApp) {
    let deadline = Instant::now() + crate::gui::testing::HANG_GUARD;
    while !matches!(app.edit, EditState::Ready(_)) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the session"
        );
        app.poll_background();
        std::thread::yield_now();
    }
}

#[test]
fn a_source_change_reloads_the_session_and_keeps_the_edits() {
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_session(&mut app);
    let way = app.edit.session().unwrap().source.extract.ways[0].clone();
    app.edit_way(|edits, _| edits.delete_way(&way, By::User))
        .unwrap();
    let file = std::fs::read(dir.join("edits/ways.toml")).unwrap();
    std::fs::copy(dir.join("osm/roads.osm.pbf"), dir.join("osm/other.osm.pbf")).unwrap();
    app.form.project.sources.osm = vec!["osm/other.osm.pbf".into()];
    app.settings_changed();
    assert!(matches!(app.edit, EditState::Loading));
    wait_for_the_session(&mut app);
    assert_eq!(std::fs::read(dir.join("edits/ways.toml")).unwrap(), file);
    assert_eq!(app.edit.session().unwrap().source.edits.ways.len(), 1);
}

#[test]
fn ctrl_r_rebuilds_after_an_edit() {
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_session(&mut app);
    wait_for_the_roads(&mut app);
    let way = app.edit.session().unwrap().source.extract.ways[0].clone();
    app.edit_way(|edits, _| edits.delete_way(&way, By::User))
        .unwrap();
    let mut h = state_harness(app);
    h.run_steps(3);
    assert!(
        h.query_by_label("Edited: press Ctrl+R to rebuild.")
            .is_some()
    );
    h.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::R);
    h.run_steps(2);
    assert!(matches!(h.state().roads, RoadsState::Generating));
    assert!(!h.state().edit.session().unwrap().dirty);
}

#[test]
fn undo_keeps_working_after_a_rebuild() {
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_session(&mut app);
    let way = app.edit.session().unwrap().source.extract.ways[0].clone();
    app.edit_way(|edits, _| edits.delete_way(&way, By::User))
        .unwrap();
    app.regenerate();
    assert!(app.edit.session().unwrap().can_undo());
    app.undo();
    assert_eq!(WayEdits::load(&dir).unwrap(), WayEdits::default());
}

#[test]
fn roads_are_built_from_edits_without_osm() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path());
    let mut edits = WayEdits::default();
    let a = edits.new_node(47.549, 12.97, By::User).unwrap();
    let b = edits.new_node(47.551, 12.97, By::User).unwrap();
    let tags = [("highway".to_string(), "track".to_string())].into();
    edits.add_way(tags, vec![a, b], By::User).unwrap();
    edits.save(&dir).unwrap();
    let mut app = app(root.path(), &dir);
    assert!(matches!(app.roads, RoadsState::Generating));
    wait_for_the_roads(&mut app);
    let RoadsState::Ready(generated) = &app.roads else {
        panic!("the roads are not ready");
    };
    assert!(
        generated
            .features
            .iter()
            .any(|f| f.id.starts_with("user/way/")),
        "{:?}",
        generated.features.iter().map(|f| &f.id).collect::<Vec<_>>()
    );
}

#[test]
fn a_damaged_edits_file_is_reported_and_blocks_edits() {
    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    std::fs::create_dir_all(dir.join("edits")).unwrap();
    std::fs::write(dir.join("edits/ways.toml"), "not toml [").unwrap();
    let mut app = app(root.path(), &dir);
    let deadline = Instant::now() + crate::gui::testing::HANG_GUARD;
    while !matches!(app.edit, EditState::Failed(_)) {
        assert!(Instant::now() < deadline, "timed out");
        app.poll_background();
        std::thread::yield_now();
    }
    let error = app
        .edit_way(|edits, _| edits.new_node(47.5, 12.9, By::User).map(|_| ()))
        .unwrap_err();
    assert!(error.starts_with("The edit cannot be made: "), "{error}");
    app.refresh_problems();
    assert!(app.problems.rows.iter().any(|p| {
        p.message
            .starts_with("The map data for editing could not be read: ")
    }));
}

#[test]
fn a_click_in_select_picks_the_source_way() {
    use crate::gui::edit::hit::{Target, hit, tolerance_m};
    use crate::gui::levels::Level;
    use crate::gui::testing::FakeViewport;
    use crate::gui::viewport::{ViewEvent, ViewFocus, Viewport, WorldPoint};

    let root = tempfile::tempdir().unwrap();
    let dir = with_roads(root.path());
    let mut app = app(root.path(), &dir);
    wait_for_the_session(&mut app);
    wait_for_the_roads(&mut app);
    let (fake, handle) = FakeViewport::new();
    let views: Vec<Box<dyn Viewport>> = vec![Box::new(fake)];
    let mut h = state_harness(app.with_views(views));
    h.key_press(egui::Key::Num4);
    h.run_steps(3);
    assert_eq!(h.state().panel.active, Level::Roads);
    let map = h.state().form.project.map_size_m();
    let focus = ViewFocus {
        center: WorldPoint {
            x: map / 2.0,
            y: map / 2.0,
        },
        span_m: 100.0,
    };
    handle.borrow_mut().focus = Some(focus);
    h.run_steps(1);
    let rect = h.state().seen_rect;
    let tol = tolerance_m(focus.span_m, f64::from(rect.width().min(rect.height())));
    // a place on a way where a built piece is under the click too
    let state = h.state();
    let session = state.edit.session().unwrap();
    let (way, at) = session
        .source
        .ways
        .ways
        .iter()
        .flat_map(|w| {
            let points: Vec<WorldPoint> = w
                .points
                .iter()
                .filter_map(|&(lat, lon)| session.source.frame.to_map(lat, lon).ok())
                .map(|(x, y)| WorldPoint { x, y })
                .collect();
            points
                .windows(2)
                .map(|p| {
                    (
                        w.clone(),
                        WorldPoint {
                            x: f64::midpoint(p[0].x, p[1].x),
                            y: f64::midpoint(p[0].y, p[1].y),
                        },
                    )
                })
                .collect::<Vec<_>>()
        })
        .find(|(w, at)| {
            hit(session, *at, tol) == Some(Target::Way(w.id))
                && crate::gui::selection::pick_at(state, *at, egui::Modifiers::NONE).is_some()
        })
        .expect("a way with a built piece under its middle");
    handle.borrow_mut().queued.push(ViewEvent::Click {
        at,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    let selection = h.state().selection.clone().expect("something is picked");
    assert_eq!(selection.target, Some(Target::Way(way.id)));
    let class = way.tags["highway"].replace('_', " ");
    let class = class[..1].to_uppercase() + &class[1..];
    assert!(selection.title.starts_with(&class), "{}", selection.title);
    assert_ne!(selection.title, "Road piece");
    let names: Vec<&str> = selection.rows.iter().map(|r| r.0.as_str()).collect();
    let built = names.iter().position(|n| *n == "Built here").unwrap();
    assert!(names[built..].contains(&"Surface"), "{names:?}");
    assert!(h.query_by_label("Built here").is_some());
    // nothing near: nothing is picked
    handle.borrow_mut().queued.push(ViewEvent::Click {
        at: WorldPoint { x: 1.0e6, y: 1.0e6 },
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    assert!(h.state().selection.is_none());
}

// ---- the tools: Move, Draw, Cut, Join, Bridge and Delete ----

mod tools {
    use egui::{Key, Modifiers};
    use naves_core::edits::{USER_ID_BASE, WayAction};

    use super::*;
    use crate::gui::edit::fixture::session as fixture_session;
    use crate::gui::levels::Level;
    use crate::gui::shortcuts::ToolId;
    use crate::gui::testing::{FAKE_LEVEL, FakeHandle, FakeViewport};
    use crate::gui::viewport::{ViewEvent, ViewFocus, Viewport, WorldPoint};

    /// Seven nodes apart, so that a click lands on one node only.
    const A: &[(f64, f64)] = &[
        (500.0, 500.0),
        (550.0, 500.0),
        (600.0, 500.0),
        (650.0, 500.0),
        (700.0, 500.0),
    ];
    const B: &[(f64, f64)] = &[(710.0, 500.0), (760.0, 500.0), (810.0, 500.0)];
    const C: &[(f64, f64)] = &[(500.0, 600.0), (600.0, 600.0)];

    fn at(x: f64, y: f64) -> WorldPoint {
        WorldPoint { x, y }
    }

    struct Setup {
        h: Harness<'static, NavesApp>,
        fake: FakeHandle,
        dir: PathBuf,
        _root: tempfile::TempDir,
    }

    fn setup(extra: bool) -> Setup {
        setup_with(extra, true)
    }

    /// With `fixture` the session holds three drawn ways; without it the project's own (empty) session.
    fn setup_with(extra: bool, fixture: bool) -> Setup {
        let root = tempfile::tempdir().unwrap();
        let dir = project(root.path());
        let mut app = app(root.path(), &dir);
        if extra {
            app = app.with_extra_level(&FAKE_LEVEL);
        }
        wait_for_the_session(&mut app);
        if fixture {
            app.edit = EditState::Ready(Box::new(fixture_session(&[(7, A), (8, B), (9, C)])));
        }
        let (fake, handle) = FakeViewport::new();
        let views: Vec<Box<dyn Viewport>> = vec![Box::new(fake)];
        let mut h = state_harness(app.with_views(views));
        step_until(&mut h, |h| {
            matches!(h.state().heightmap, HeightmapState::Ready(_))
        });
        handle.borrow_mut().focus = Some(ViewFocus {
            center: at(650.0, 550.0),
            span_m: 100.0,
        });
        h.run_steps(3);
        Setup {
            h,
            fake: handle,
            dir,
            _root: root,
        }
    }

    impl Setup {
        fn roads(&mut self) {
            self.h.key_press(Key::Num4);
            self.h.run_steps(3);
            assert_eq!(self.h.state().panel.active, Level::Roads);
        }

        fn key(&mut self, key: Key) {
            self.h.key_press(key);
            self.h.run_steps(3);
        }

        fn key_with(&mut self, modifiers: Modifiers, key: Key) {
            self.h.key_press_modifiers(modifiers, key);
            self.h.run_steps(3);
        }

        fn send(&mut self, events: &[ViewEvent]) {
            self.fake.borrow_mut().queued.extend_from_slice(events);
            self.h.run_steps(3);
        }

        fn click(&mut self, x: f64, y: f64) {
            self.send(&[ViewEvent::Click {
                at: at(x, y),
                modifiers: Modifiers::NONE,
            }]);
        }

        fn clicks(&mut self, points: &[(f64, f64)]) {
            for &(x, y) in points {
                self.click(x, y);
            }
        }

        fn saved(&self) -> WayEdits {
            WayEdits::load(&self.dir).unwrap()
        }

        fn file(&self) -> PathBuf {
            self.dir.join("edits/ways.toml")
        }

        fn session(&self) -> &EditSession {
            self.h.state().edit.session().unwrap()
        }

        fn pick_class(&mut self, name: &str) {
            self.h.get_by_label(name).click();
            self.h.run_steps(3);
        }

        fn geo(&self, x: f64, y: f64) -> (f64, f64) {
            self.session().source.frame.to_geo(x, y).unwrap()
        }
    }

    fn tag<'a>(edit: &'a naves_core::edits::WayEdit, key: &str) -> Option<&'a str> {
        edit.tags.get(key).map(String::as_str)
    }

    fn osm(ids: &[i64]) -> Vec<ElementId> {
        ids.iter()
            .map(|&i| ElementId::from_raw(i).unwrap())
            .collect()
    }

    #[test]
    fn move_drags_a_node() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::G);
        assert_eq!(s.h.state().tool, ToolId::MOVE);
        s.send(&[
            ViewEvent::Press {
                at: at(550.0, 500.0),
                modifiers: Modifiers::NONE,
            },
            ViewEvent::Drag {
                at: at(551.0, 510.0),
                modifiers: Modifiers::NONE,
            },
            ViewEvent::Drag {
                at: at(552.0, 530.0),
                modifiers: Modifiers::NONE,
            },
            ViewEvent::Release {
                at: at(552.0, 530.0),
                modifiers: Modifiers::NONE,
            },
        ]);
        let saved = s.saved();
        assert_eq!(saved.nodes.len(), 1);
        let node = &saved.nodes[0];
        assert_eq!(node.id, ElementId::Osm(701));
        let (lat, lon) = s.geo(552.0, 530.0);
        assert!((node.lat - lat).abs() < 1e-9 && (node.lon - lon).abs() < 1e-9);
        let (base_lat, base_lon) = s.geo(550.0, 500.0);
        let base = node.base.expect("the OSM position is kept");
        assert!((base.0 - base_lat).abs() < 1e-9 && (base.1 - base_lon).abs() < 1e-9);
        assert!(s.session().can_undo());
        assert_eq!(saved.ways.len(), 0);
    }

    #[test]
    fn esc_during_a_drag_moves_nothing() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::G);
        s.send(&[
            ViewEvent::Press {
                at: at(550.0, 500.0),
                modifiers: Modifiers::NONE,
            },
            ViewEvent::Drag {
                at: at(552.0, 530.0),
                modifiers: Modifiers::NONE,
            },
        ]);
        s.key(Key::Escape);
        s.send(&[ViewEvent::Release {
            at: at(552.0, 530.0),
            modifiers: Modifiers::NONE,
        }]);
        assert!(!s.file().exists());
        assert!(!s.session().can_undo());
        assert_eq!(
            s.h.state().tool,
            ToolId::MOVE,
            "Esc cancelled the drag only"
        );
    }

    #[test]
    fn draw_adds_a_way_with_the_class_tags() {
        let mut s = setup(false);
        s.roads();
        s.pick_class("Secondary");
        assert_eq!(s.h.state().tool, ToolId::LINE);
        s.clicks(&[(520.0, 540.0), (560.0, 560.0), (600.0, 540.0)]);
        assert_eq!(s.h.state().drawing.points.len(), 3);
        assert!(!s.file().exists(), "nothing is saved before the line ends");
        s.key(Key::Space);
        let saved = s.saved();
        assert_eq!(saved.ways.len(), 1);
        let way = &saved.ways[0];
        assert_eq!(way.action, WayAction::Add);
        assert_eq!(tag(way, "highway"), Some("secondary"));
        assert_eq!(tag(way, "naves:keep"), Some("yes"));
        assert_eq!(way.nodes.len(), 3);
        assert!(way.nodes.iter().all(|n| n.is_user()));
        assert_eq!(saved.nodes.len(), 3);
        assert_eq!(s.h.state().drawing.points.len(), 0);
        assert!(s.session().can_undo());
    }

    #[test]
    fn draw_save_rebuild() {
        let mut s = setup_with(false, false);
        s.roads();
        s.pick_class("Secondary");
        s.clicks(&[(300.0, 300.0), (500.0, 400.0), (700.0, 400.0)]);
        s.key(Key::Space);
        assert_eq!(s.saved().ways.len(), 1);
        s.key_with(Modifiers::CTRL, Key::R);
        step_until(&mut s.h, |h| {
            !matches!(h.state().roads, RoadsState::Generating)
        });
        let RoadsState::Ready(generated) = &s.h.state().roads else {
            panic!("the roads are not ready");
        };
        let roads: Vec<_> = generated
            .features
            .iter()
            .filter(|f| f.id.starts_with("user/way/"))
            .collect();
        assert_eq!(
            roads.len(),
            1,
            "{:?}",
            generated.features.iter().map(|f| &f.id).collect::<Vec<_>>()
        );
        assert!(roads[0].id.ends_with("/0"), "{}", roads[0].id);
        // held: the way is kept as drawn (the build's `Stretch::held` comes from this tag)
        assert_eq!(tag(&s.saved().ways[0], "naves:keep"), Some("yes"));
        assert_ne!(generated.pieces.len(), 0);
    }

    #[test]
    fn draw_without_a_class_is_a_residential_street() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::C);
        s.clicks(&[(520.0, 540.0), (560.0, 560.0)]);
        s.key(Key::Enter);
        let saved = s.saved();
        assert_eq!(tag(&saved.ways[0], "highway"), Some("residential"));
    }

    #[test]
    fn draw_joins_an_existing_node() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::C);
        s.clicks(&[(500.0, 500.0), (500.0, 550.0), (480.0, 580.0)]);
        s.key(Key::Space);
        let saved = s.saved();
        let way = &saved.ways[0];
        assert_eq!(way.nodes[0], ElementId::Osm(700));
        assert!(way.nodes[1].is_user());
        assert_eq!(saved.nodes.len(), 2, "no node edit for the node it joins");
        assert!(saved.nodes.iter().all(|n| n.id != ElementId::Osm(700)));
    }

    #[test]
    fn ctrl_z_takes_a_point_back() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::C);
        s.clicks(&[(520.0, 540.0), (560.0, 560.0), (600.0, 540.0)]);
        s.key_with(Modifiers::CTRL, Key::Z);
        assert_eq!(s.h.state().drawing.points.len(), 2);
        s.key(Key::Backspace);
        assert_eq!(s.h.state().drawing.points.len(), 1);
        assert!(!s.file().exists(), "taking a point back saves nothing");
        assert!(!s.session().can_undo());
    }

    #[test]
    fn ctrl_z_while_drawing_leaves_a_saved_edit_alone() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::R);
        s.click(550.0, 500.0);
        let after_cut = std::fs::read(s.file()).unwrap();
        s.key(Key::C);
        s.clicks(&[(520.0, 540.0), (560.0, 560.0)]);
        s.key_with(Modifiers::CTRL, Key::Z);
        assert_eq!(s.h.state().drawing.points.len(), 1);
        assert_eq!(std::fs::read(s.file()).unwrap(), after_cut);
        // with nothing being drawn Ctrl+Z undoes the edit again
        s.key(Key::Escape);
        s.key_with(Modifiers::CTRL, Key::Z);
        assert_eq!(s.saved(), WayEdits::default());
    }

    #[test]
    fn space_with_one_point_draws_nothing() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::C);
        s.click(520.0, 540.0);
        s.key(Key::Space);
        assert!(!s.file().exists());
        assert_eq!(s.h.state().drawing.points.len(), 1, "the point stays");
    }

    #[test]
    fn esc_discards_the_line() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::C);
        s.clicks(&[(520.0, 540.0), (560.0, 560.0)]);
        s.key(Key::Escape);
        assert_eq!(s.h.state().drawing.points.len(), 0);
        assert!(!s.file().exists());
    }

    #[test]
    fn right_click_and_double_click_finish() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::C);
        s.clicks(&[(520.0, 540.0), (560.0, 560.0)]);
        s.send(&[ViewEvent::SecondaryClick {
            at: at(560.0, 560.0),
            modifiers: Modifiers::NONE,
        }]);
        assert_eq!(s.saved().ways.len(), 1);
        // a double click comes as two clicks and the double click; the doubled point is one point
        s.send(&[
            ViewEvent::Click {
                at: at(520.0, 570.0),
                modifiers: Modifiers::NONE,
            },
            ViewEvent::Click {
                at: at(580.0, 590.0),
                modifiers: Modifiers::NONE,
            },
            ViewEvent::Click {
                at: at(580.0, 590.0),
                modifiers: Modifiers::NONE,
            },
            ViewEvent::DoubleClick {
                at: at(580.0, 590.0),
                modifiers: Modifiers::NONE,
            },
        ]);
        let saved = s.saved();
        assert_eq!(saved.ways.len(), 2);
        assert_eq!(saved.ways[1].nodes.len(), 2);
    }

    #[test]
    fn cut_splits_a_way_in_one_undo_step() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::R);
        assert_eq!(s.h.state().tool, ToolId::CUT);
        s.click(600.0, 500.0);
        let saved = s.saved();
        assert_eq!(saved.ways.len(), 2);
        let first = saved
            .ways
            .iter()
            .find(|w| w.id == ElementId::Osm(7))
            .unwrap();
        assert_eq!(first.action, WayAction::Modify);
        assert_eq!(first.nodes, osm(&[700, 701, 702]));
        let second = saved.ways.iter().find(|w| w.id.is_user()).unwrap();
        assert_eq!(second.action, WayAction::Add);
        assert_eq!(second.nodes, osm(&[702, 703, 704]));
        assert_eq!(tag(second, "highway"), Some("residential"));
        assert_eq!(s.session().source.ways.ways.len(), 4);
        s.key_with(Modifiers::CTRL, Key::Z);
        assert_eq!(s.saved(), WayEdits::default());
        assert_eq!(s.session().source.ways.ways.len(), 3);
    }

    #[test]
    fn cut_at_an_end_node_does_nothing() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::R);
        s.click(500.0, 500.0);
        assert!(!s.file().exists());
    }

    #[test]
    fn join_with_the_same_tags_makes_one_way() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::T);
        assert_eq!(s.h.state().tool, ToolId::JOIN);
        s.clicks(&[(700.0, 500.0), (710.0, 500.0)]);
        let saved = s.saved();
        let joined = saved
            .ways
            .iter()
            .find(|w| w.id == ElementId::Osm(7))
            .unwrap();
        assert_eq!(joined.action, WayAction::Modify);
        assert_eq!(joined.nodes, osm(&[700, 701, 702, 703, 704, 801, 802]));
        let gone = saved
            .ways
            .iter()
            .find(|w| w.id == ElementId::Osm(8))
            .unwrap();
        assert_eq!(gone.action, WayAction::Delete);
        assert_eq!(s.session().source.ways.ways.len(), 2);
        s.key_with(Modifiers::CTRL, Key::Z);
        assert_eq!(s.saved(), WayEdits::default(), "one undo step");
    }

    #[test]
    fn join_with_other_tags_connects_the_ends() {
        let mut s = setup(false);
        let b = s.session().original(8).unwrap().clone();
        let tags = [("highway".to_string(), "track".to_string())].into();
        s.h.state_mut()
            .edit_way(|edits, _| edits.modify_way(&b, tags, osm(&[800, 801, 802]), By::User))
            .unwrap();
        s.roads();
        s.key(Key::T);
        s.clicks(&[(700.0, 500.0), (710.0, 500.0)]);
        let saved = s.saved();
        assert_eq!(
            saved.ways.len(),
            1,
            "both ways are kept, A is not even edited"
        );
        let second = &saved.ways[0];
        assert_eq!(second.id, ElementId::Osm(8));
        assert_eq!(second.action, WayAction::Modify);
        assert_eq!(second.nodes, osm(&[704, 801, 802]));
        assert_eq!(tag(second, "highway"), Some("track"));
        assert_eq!(s.session().source.ways.ways.len(), 3);
    }

    #[test]
    fn join_ignores_an_inner_node() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::T);
        s.clicks(&[(600.0, 500.0), (710.0, 500.0)]);
        assert!(!s.file().exists());
    }

    #[test]
    fn bridge_splits_out_the_part_and_tags_it() {
        let mut s = setup(false);
        s.roads();
        s.key_with(Modifiers::SHIFT, Key::T);
        assert_eq!(s.h.state().tool, ToolId::BRIDGE);
        s.clicks(&[(550.0, 500.0), (650.0, 500.0)]);
        let saved = s.saved();
        assert_eq!(saved.ways.len(), 3);
        let before = saved
            .ways
            .iter()
            .find(|w| w.id == ElementId::Osm(7))
            .unwrap();
        assert_eq!(before.nodes, osm(&[700, 701]));
        assert_eq!(tag(before, "bridge"), None);
        let added: Vec<_> = saved.ways.iter().filter(|w| w.id.is_user()).collect();
        assert_eq!(added.len(), 2);
        let bridge = added.iter().find(|w| tag(w, "bridge").is_some()).unwrap();
        assert_eq!(bridge.nodes, osm(&[701, 702, 703]));
        assert_eq!(tag(bridge, "bridge"), Some("yes"));
        assert_eq!(tag(bridge, "layer"), Some("1"));
        let after = added.iter().find(|w| tag(w, "bridge").is_none()).unwrap();
        assert_eq!(after.nodes, osm(&[703, 704]));
        s.key_with(Modifiers::CTRL, Key::Z);
        assert_eq!(s.saved(), WayEdits::default(), "one undo step");
    }

    #[test]
    fn bridge_on_a_bridge_untags_it() {
        let mut s = setup(false);
        s.roads();
        s.key_with(Modifiers::SHIFT, Key::T);
        s.clicks(&[(550.0, 500.0), (650.0, 500.0)]);
        // a node inside the bridge: one click
        s.click(600.0, 500.0);
        let saved = s.saved();
        let bridge = saved
            .ways
            .iter()
            .find(|w| w.nodes == osm(&[701, 702, 703]))
            .unwrap();
        assert_eq!(tag(bridge, "bridge"), None);
        assert_eq!(tag(bridge, "layer"), None);
        assert_eq!(tag(bridge, "highway"), Some("residential"));
    }

    #[test]
    fn delete_removes_the_selected_way() {
        let mut s = setup(false);
        s.roads();
        s.click(525.0, 500.0); // Select: a way
        assert!(s.h.state().selection.is_some());
        s.key(Key::X);
        let saved = s.saved();
        assert_eq!(saved.ways.len(), 1);
        assert_eq!(saved.ways[0].id, ElementId::Osm(7));
        assert_eq!(saved.ways[0].action, WayAction::Delete);
        assert!(s.session().way(7).is_none());
        assert!(s.h.state().selection.is_none());
        s.key_with(Modifiers::CTRL, Key::Z);
        assert!(s.session().way(7).is_some());
    }

    #[test]
    fn the_delete_key_deletes_too() {
        let mut s = setup(false);
        s.roads();
        s.click(525.0, 600.0);
        s.key(Key::Delete);
        assert_eq!(s.saved().ways[0].id, ElementId::Osm(9));
    }

    #[test]
    fn delete_takes_a_drawn_node_out() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::C);
        s.clicks(&[(520.0, 540.0), (560.0, 560.0), (600.0, 540.0)]);
        s.key(Key::Space);
        s.key(Key::Escape); // back to Select
        assert_eq!(s.h.state().tool, ToolId::SELECT);
        s.click(560.0, 560.0);
        s.key(Key::X);
        let saved = s.saved();
        assert_eq!(saved.ways.len(), 1);
        assert_eq!(saved.ways[0].nodes.len(), 2);
        // a way left with one node is deleted
        s.click(520.0, 540.0);
        s.key(Key::X);
        let saved = s.saved();
        assert!(saved.ways.is_empty(), "{saved:?}");
        assert_eq!(saved.nodes.len(), 0);
    }

    #[test]
    fn an_osm_node_is_not_deleted() {
        let mut s = setup(false);
        s.roads();
        s.click(550.0, 500.0);
        s.key(Key::X);
        assert!(!s.file().exists());
        assert!(s.session().way(7).is_some());
    }

    #[test]
    fn delete_does_nothing_without_a_selection() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::X);
        assert!(!s.file().exists());
    }

    #[test]
    fn a_click_on_nothing_changes_nothing() {
        let mut s = setup(false);
        s.roads();
        // an edit first, so that there is a file, a stamp and an undo step
        s.key(Key::R);
        s.click(550.0, 500.0);
        let bytes = std::fs::read(s.file()).unwrap();
        let stamp = std::fs::metadata(s.file()).unwrap().modified().unwrap();
        let ways = s.session().source.ways.ways.len();
        let status = s.h.state().status_text();
        for (modifiers, key) in [
            (Modifiers::NONE, Key::R),
            (Modifiers::NONE, Key::T),
            (Modifiers::SHIFT, Key::T),
            (Modifiers::NONE, Key::G),
        ] {
            s.key_with(modifiers, key);
            let tool_status = s.h.state().status_text();
            s.send(&[
                ViewEvent::Press {
                    at: at(1.0e5, 1.0e5),
                    modifiers: Modifiers::NONE,
                },
                ViewEvent::Release {
                    at: at(1.0e5, 1.0e5),
                    modifiers: Modifiers::NONE,
                },
            ]);
            s.click(1.0e5, 1.0e5);
            s.click(1.0e5, 1.0e5);
            assert_eq!(std::fs::read(s.file()).unwrap(), bytes);
            assert_eq!(
                std::fs::metadata(s.file()).unwrap().modified().unwrap(),
                stamp
            );
            assert_eq!(s.session().source.ways.ways.len(), ways);
            assert_eq!(s.h.state().status_text(), tool_status);
        }
        assert_eq!(status, "Edited: press Ctrl+R to rebuild.");
        // one undo step is all there is
        s.key(Key::Escape);
        s.key_with(Modifiers::CTRL, Key::Z);
        assert_eq!(s.saved(), WayEdits::default());
        assert!(!s.session().can_undo());
    }

    #[test]
    fn the_roads_tools_work_from_another_level() {
        let mut s = setup(true);
        s.h.key_press(Key::Num6);
        s.h.run_steps(3);
        assert_eq!(s.h.state().panel.active, Level::Buildings);
        s.h.get_by_label("Everything").click();
        s.h.run_steps(3);
        s.h.state_mut().palette.search = "secondary".to_string();
        s.h.run_steps(3);
        s.pick_class("Secondary");
        assert_eq!(s.h.state().panel.active, Level::Buildings);
        assert_eq!(s.h.state().item_level, Some(Level::Roads));
        assert_eq!(s.h.state().tool, ToolId::LINE);
        assert!(
            s.h.query_by_label("C Draw").is_some(),
            "the rail lists Draw"
        );
        s.clicks(&[(520.0, 540.0), (560.0, 560.0), (600.0, 540.0)]);
        s.key(Key::Space);
        let saved = s.saved();
        assert_eq!(saved.ways.len(), 1);
        assert_eq!(tag(&saved.ways[0], "highway"), Some("secondary"));
        assert!(saved.ways[0].id.raw().unwrap() >= USER_ID_BASE);
    }

    // ---- the properties of a selected way ----

    use crate::gui::edit::properties::{WayChange, class_label, class_of, origin_line, way_title};

    fn tags_of(s: &Setup, raw: i64) -> std::collections::BTreeMap<String, String> {
        s.session().way(raw).unwrap().tags.clone()
    }

    #[test]
    fn the_origin_line_says_where_a_way_came_from() {
        assert_eq!(origin_line(Origin::Osm, 8421), "OSM way 8421, unchanged");
        assert_eq!(
            origin_line(Origin::EditedOsm, 8421),
            "OSM way 8421, edited by you"
        );
        assert_eq!(origin_line(Origin::Drawn, USER_ID_BASE), "Drawn by you");
        // the picked way says the same
        let mut s = setup(false);
        s.roads();
        s.click(525.0, 500.0);
        let selection = s.h.state().selection.clone().unwrap();
        assert_eq!(selection.origin.as_deref(), Some("OSM way 7, unchanged"));
    }

    #[test]
    fn changing_the_class_retags_the_way() {
        let mut s = setup(false);
        let app = s.h.state_mut();
        app.change_way(7, &WayChange::Class("secondary")).unwrap();
        app.change_way(7, &WayChange::Surface(Some("asphalt")))
            .unwrap();
        app.change_way(7, &WayChange::Class("track2")).unwrap();
        let saved = s.saved();
        let way = saved
            .ways
            .iter()
            .find(|w| w.id == ElementId::Osm(7))
            .unwrap();
        assert_eq!(tag(way, "highway"), Some("track"));
        assert_eq!(tag(way, "tracktype"), Some("grade2"));
        assert_eq!(tag(way, "surface"), Some("asphalt"));
        // and back: the track type goes with the old class
        s.h.state_mut()
            .change_way(7, &WayChange::Class("primary"))
            .unwrap();
        let tags = tags_of(&s, 7);
        assert_eq!(tags.get("highway").map(String::as_str), Some("primary"));
        assert!(!tags.contains_key("tracktype"));
        assert_eq!(way_title(&tags), "Primary");
    }

    #[test]
    fn keep_and_plan_are_switches() {
        let mut s = setup(false);
        let app = s.h.state_mut();
        app.change_way(7, &WayChange::Keep(true)).unwrap();
        app.change_way(7, &WayChange::Keep(false)).unwrap();
        app.change_way(7, &WayChange::Plan(true)).unwrap();
        let saved = s.saved();
        let way = &saved.ways[0];
        assert_eq!(tag(way, "naves:keep"), Some("no"));
        assert_eq!(tag(way, "naves:plan"), Some("yes"));
        s.h.state_mut()
            .change_way(7, &WayChange::Plan(false))
            .unwrap();
        assert!(!tags_of(&s, 7).contains_key("naves:plan"));
    }

    #[test]
    fn bridge_layer_and_surface_write_their_tags() {
        let mut s = setup(false);
        let app = s.h.state_mut();
        app.change_way(7, &WayChange::Bridge(true)).unwrap();
        let tags = tags_of(&s, 7);
        assert_eq!(tags.get("bridge").map(String::as_str), Some("yes"));
        assert_eq!(tags.get("layer").map(String::as_str), Some("1"));
        let app = s.h.state_mut();
        app.change_way(7, &WayChange::Layer(9)).unwrap();
        assert_eq!(tags_of(&s, 7).get("layer").map(String::as_str), Some("5"));
        let app = s.h.state_mut();
        app.change_way(7, &WayChange::Layer(0)).unwrap();
        app.change_way(7, &WayChange::Bridge(false)).unwrap();
        let tags = tags_of(&s, 7);
        assert!(!tags.contains_key("layer") && !tags.contains_key("bridge"));
        s.h.state_mut()
            .change_way(7, &WayChange::Surface(Some("gravel")))
            .unwrap();
        s.h.state_mut()
            .change_way(7, &WayChange::Surface(None))
            .unwrap();
        assert!(!tags_of(&s, 7).contains_key("surface"));
    }

    #[test]
    fn reset_to_osm_removes_the_edit() {
        let mut s = setup(false);
        s.h.state_mut()
            .change_way(7, &WayChange::Class("track2"))
            .unwrap();
        assert_eq!(s.session().origin(7), Origin::EditedOsm);
        s.h.state_mut().change_way(7, &WayChange::Reset).unwrap();
        assert_eq!(s.saved(), WayEdits::default());
        assert_eq!(s.session().origin(7), Origin::Osm);
        assert_eq!(
            origin_line(s.session().origin(7), 7),
            "OSM way 7, unchanged"
        );
        s.key_with(Modifiers::CTRL, Key::Z);
        assert_eq!(s.session().origin(7), Origin::EditedOsm, "one undo step");
    }

    #[test]
    fn reset_to_osm_puts_the_moved_nodes_back() {
        let mut s = setup(false);
        s.roads();
        s.key(Key::G);
        s.send(&[
            ViewEvent::Press {
                at: at(550.0, 500.0),
                modifiers: Modifiers::NONE,
            },
            ViewEvent::Release {
                at: at(552.0, 530.0),
                modifiers: Modifiers::NONE,
            },
        ]);
        assert_eq!(s.session().origin(7), Origin::EditedOsm);
        s.h.state_mut().change_way(7, &WayChange::Reset).unwrap();
        assert_eq!(s.saved(), WayEdits::default());
        assert_eq!(s.session().origin(7), Origin::Osm);
        s.key_with(Modifiers::CTRL, Key::Z);
        assert_eq!(s.session().origin(7), Origin::EditedOsm, "one undo step");
    }

    #[test]
    fn an_unknown_highway_is_shown_as_other() {
        let tags = [("highway".to_string(), "cycleway".to_string())].into();
        assert!(class_of(&tags).is_none());
        assert_eq!(class_label(&tags), "Other: highway=cycleway");
        assert_eq!(way_title(&tags), "Road");
        let tags = [
            ("highway".to_string(), "track".to_string()),
            ("tracktype".to_string(), "grade3".to_string()),
        ]
        .into();
        assert_eq!(class_label(&tags), "Track, grade 3");
    }
}
