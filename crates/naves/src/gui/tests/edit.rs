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
