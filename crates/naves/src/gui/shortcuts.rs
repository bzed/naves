// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The one table of default keyboard shortcuts, and the rules for when a key fires.
//!
//! The table holds **every** shortcut of the editor's design, also those of what is not built yet, so that double
//! assignments and the stable ids are checked for the final set from the start. [`action_exists`] says what can
//! be done today; a key whose action does not exist does nothing.

use std::collections::BTreeMap;
use std::fmt;

use eframe::egui::{self, InputState, Key, Modifiers};

use super::NavesApp;
use super::levels::Level;

/// A tool of the rail, by a stable name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ToolId(pub &'static str);

impl ToolId {
    pub const SELECT: ToolId = ToolId("select");
    pub const MOVE: ToolId = ToolId("move");
    pub const ROTATE: ToolId = ToolId("rotate");
    pub const SCALE: ToolId = ToolId("scale");
    pub const ADD: ToolId = ToolId("add");
    pub const LINE: ToolId = ToolId("line");
    pub const AREA: ToolId = ToolId("area");
    pub const BRUSH: ToolId = ToolId("brush");
    pub const SMOOTH: ToolId = ToolId("smooth");
    pub const FLATTEN: ToolId = ToolId("flatten");
    pub const RAMP: ToolId = ToolId("ramp");
    pub const HOLE: ToolId = ToolId("hole");
    pub const SOFTEN: ToolId = ToolId("soften");
    pub const CUT: ToolId = ToolId("cut");
    pub const JOIN: ToolId = ToolId("join");
    pub const BRIDGE: ToolId = ToolId("bridge");
    pub const COMPOSITION: ToolId = ToolId("composition");
    pub const ENTRANCE: ToolId = ToolId("entrance");
    pub const BUNKER_HOLE: ToolId = ToolId("bunker_hole");
}

/// What a key can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Go to a level.
    Level(Level),
    /// Pick a tool.
    Tool(ToolId),
    /// Hide the active level.
    HideLevel,
    /// Show the other levels in full.
    ShowOthersInFull,
    /// Switch between the map and the 3D view.
    SwitchView,
    /// Frame the selection.
    FrameSelection,
    /// Frame the whole map.
    FrameMap,
    /// World or local axes.
    WorldAxes,
    /// Follow the ground.
    FollowGround,
    /// Delete the selection.
    Delete,
    /// Duplicate the selection.
    Duplicate,
    /// Undo.
    Undo,
    /// Redo.
    Redo,
    /// Search the items.
    SearchItems,
    /// A smaller brush.
    BrushSmaller,
    /// A larger brush.
    BrushLarger,
    /// A weaker brush.
    BrushWeaker,
    /// A stronger brush.
    BrushStronger,
    /// Finish a line or an area.
    Finish,
    /// Cancel and go back to Select.
    Cancel,
    /// Regenerate the active level in the changed area.
    Regenerate,
    /// Build the project.
    Build,
    /// Go to the next problem.
    NextProblem,
    /// Go to the previous problem.
    PreviousProblem,
    /// Show the shortcuts.
    Help,
    /// Open the settings.
    Settings,
}

/// Where a shortcut applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// In every level.
    Everywhere,
    /// Only in one level.
    Level(Level),
}

/// A key with its modifiers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Binding {
    /// The logical key.
    pub key: Key,
    /// The modifier keys, exactly.
    pub modifiers: Modifiers,
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.ctrl || self.modifiers.command {
            write!(f, "Ctrl+")?;
        }
        if self.modifiers.alt {
            write!(f, "Alt+")?;
        }
        if self.modifiers.shift {
            write!(f, "Shift+")?;
        }
        match self.key {
            Key::Escape => write!(f, "Esc"),
            key => write!(f, "{}", key.symbol_or_name()),
        }
    }
}

impl Binding {
    /// Reads `Ctrl+Shift+B`, `F9`, `,` and the like. `None` for a key name that egui does not know.
    #[must_use]
    pub fn parse(text: &str) -> Option<Binding> {
        let mut rest = text.trim();
        let mut modifiers = Modifiers::NONE;
        'prefixes: loop {
            for prefix in ["ctrl+", "alt+", "shift+"] {
                if let Some(head) = rest.get(..prefix.len())
                    && head.eq_ignore_ascii_case(prefix)
                    && let Some(tail) = rest.get(prefix.len()..)
                    && !tail.is_empty()
                {
                    match prefix {
                        "ctrl+" => modifiers.ctrl = true,
                        "alt+" => modifiers.alt = true,
                        _ => modifiers.shift = true,
                    }
                    rest = tail;
                    continue 'prefixes;
                }
            }
            break;
        }
        Key::from_name(rest).map(|key| Binding { key, modifiers })
    }
}

/// One row of the table.
#[derive(Debug, Clone, Copy)]
pub struct Shortcut {
    /// Stable forever: the settings file refers to it.
    pub id: &'static str,
    /// The default key.
    pub binding: Binding,
    /// What it does.
    pub action: Action,
    /// How it is called, in the sheet, the manual and the settings.
    pub label: &'static str,
    /// Where it applies.
    pub scope: Scope,
}

const fn row(
    id: &'static str,
    key: Key,
    modifiers: Modifiers,
    action: Action,
    label: &'static str,
    scope: Scope,
) -> Shortcut {
    Shortcut {
        id,
        binding: Binding { key, modifiers },
        action,
        label,
        scope,
    }
}

/// Every default shortcut. Common tools are `Everywhere`; the keys a level has of its own are per level.
pub const SHORTCUTS: &[Shortcut] = &[
    row(
        "level.project",
        Key::Num0,
        Modifiers::NONE,
        Action::Level(Level::Project),
        "Go to Project",
        Scope::Everywhere,
    ),
    row(
        "level.terrain",
        Key::Num1,
        Modifiers::NONE,
        Action::Level(Level::Terrain),
        "Go to Terrain",
        Scope::Everywhere,
    ),
    row(
        "level.ground",
        Key::Num2,
        Modifiers::NONE,
        Action::Level(Level::Ground),
        "Go to Ground",
        Scope::Everywhere,
    ),
    row(
        "level.water",
        Key::Num3,
        Modifiers::NONE,
        Action::Level(Level::Water),
        "Go to Water",
        Scope::Everywhere,
    ),
    row(
        "level.roads",
        Key::Num4,
        Modifiers::NONE,
        Action::Level(Level::Roads),
        "Go to Roads and paths",
        Scope::Everywhere,
    ),
    row(
        "level.vegetation",
        Key::Num5,
        Modifiers::NONE,
        Action::Level(Level::Vegetation),
        "Go to Vegetation",
        Scope::Everywhere,
    ),
    row(
        "level.buildings",
        Key::Num6,
        Modifiers::NONE,
        Action::Level(Level::Buildings),
        "Go to Buildings",
        Scope::Everywhere,
    ),
    row(
        "level.objects",
        Key::Num7,
        Modifiers::NONE,
        Action::Level(Level::Objects),
        "Go to Objects",
        Scope::Everywhere,
    ),
    row(
        "level.life",
        Key::Num8,
        Modifiers::NONE,
        Action::Level(Level::Life),
        "Go to Life",
        Scope::Everywhere,
    ),
    row(
        "level.economy",
        Key::Num9,
        Modifiers::NONE,
        Action::Level(Level::Economy),
        "Go to Economy",
        Scope::Everywhere,
    ),
    row(
        "tool.select",
        Key::Q,
        Modifiers::NONE,
        Action::Tool(ToolId::SELECT),
        "Select",
        Scope::Everywhere,
    ),
    row(
        "tool.move",
        Key::W,
        Modifiers::NONE,
        Action::Tool(ToolId::MOVE),
        "Move",
        Scope::Everywhere,
    ),
    row(
        "tool.rotate",
        Key::E,
        Modifiers::NONE,
        Action::Tool(ToolId::ROTATE),
        "Rotate",
        Scope::Everywhere,
    ),
    row(
        "tool.scale",
        Key::R,
        Modifiers::NONE,
        Action::Tool(ToolId::SCALE),
        "Scale",
        Scope::Everywhere,
    ),
    row(
        "tool.add",
        Key::A,
        Modifiers::NONE,
        Action::Tool(ToolId::ADD),
        "Add as a point",
        Scope::Everywhere,
    ),
    row(
        "tool.line",
        Key::N,
        Modifiers::NONE,
        Action::Tool(ToolId::LINE),
        "Line",
        Scope::Everywhere,
    ),
    row(
        "tool.area",
        Key::L,
        Modifiers::NONE,
        Action::Tool(ToolId::AREA),
        "Area",
        Scope::Everywhere,
    ),
    row(
        "tool.brush",
        Key::B,
        Modifiers::NONE,
        Action::Tool(ToolId::BRUSH),
        "Brush",
        Scope::Everywhere,
    ),
    row(
        "tool.terrain.smooth",
        Key::S,
        Modifiers::NONE,
        Action::Tool(ToolId::SMOOTH),
        "Smooth",
        Scope::Level(Level::Terrain),
    ),
    row(
        "tool.terrain.flatten",
        Key::T,
        Modifiers::NONE,
        Action::Tool(ToolId::FLATTEN),
        "Flatten",
        Scope::Level(Level::Terrain),
    ),
    row(
        "tool.terrain.ramp",
        Key::M,
        Modifiers::NONE,
        Action::Tool(ToolId::RAMP),
        "Ramp",
        Scope::Level(Level::Terrain),
    ),
    row(
        "tool.terrain.hole",
        Key::O,
        Modifiers::NONE,
        Action::Tool(ToolId::HOLE),
        "Hole",
        Scope::Level(Level::Terrain),
    ),
    row(
        "tool.ground.soften",
        Key::S,
        Modifiers::NONE,
        Action::Tool(ToolId::SOFTEN),
        "Soften an edge",
        Scope::Level(Level::Ground),
    ),
    row(
        "tool.roads.cut",
        Key::K,
        Modifiers::NONE,
        Action::Tool(ToolId::CUT),
        "Cut",
        Scope::Level(Level::Roads),
    ),
    row(
        "tool.roads.join",
        Key::J,
        Modifiers::NONE,
        Action::Tool(ToolId::JOIN),
        "Join",
        Scope::Level(Level::Roads),
    ),
    row(
        "tool.roads.bridge",
        Key::G,
        Modifiers::NONE,
        Action::Tool(ToolId::BRIDGE),
        "Mark a bridge",
        Scope::Level(Level::Roads),
    ),
    row(
        "tool.vegetation.cut",
        Key::K,
        Modifiers::NONE,
        Action::Tool(ToolId::CUT),
        "Cut out",
        Scope::Level(Level::Vegetation),
    ),
    row(
        "tool.buildings.composition",
        Key::C,
        Modifiers::NONE,
        Action::Tool(ToolId::COMPOSITION),
        "Make a composition",
        Scope::Level(Level::Buildings),
    ),
    row(
        "tool.buildings.entrance",
        Key::G,
        Modifiers::NONE,
        Action::Tool(ToolId::ENTRANCE),
        "Tag an entrance",
        Scope::Level(Level::Buildings),
    ),
    row(
        "tool.buildings.hole",
        Key::O,
        Modifiers::NONE,
        Action::Tool(ToolId::BUNKER_HOLE),
        "Hole for a bunker",
        Scope::Level(Level::Buildings),
    ),
    row(
        "tool.objects.composition",
        Key::C,
        Modifiers::NONE,
        Action::Tool(ToolId::COMPOSITION),
        "Make a composition",
        Scope::Level(Level::Objects),
    ),
    row(
        "level.hide",
        Key::H,
        Modifiers::NONE,
        Action::HideLevel,
        "Hide the active level",
        Scope::Everywhere,
    ),
    row(
        "level.show_others",
        Key::H,
        Modifiers::SHIFT,
        Action::ShowOthersInFull,
        "Show the other levels in full",
        Scope::Everywhere,
    ),
    row(
        "view.switch",
        Key::Tab,
        Modifiers::CTRL,
        Action::SwitchView,
        "Switch between map and 3D view",
        Scope::Everywhere,
    ),
    row(
        "frame.selection",
        Key::F,
        Modifiers::NONE,
        Action::FrameSelection,
        "Frame the selection",
        Scope::Everywhere,
    ),
    row(
        "frame.map",
        Key::F,
        Modifiers::SHIFT,
        Action::FrameMap,
        "Frame the whole map",
        Scope::Everywhere,
    ),
    row(
        "axes.world",
        Key::X,
        Modifiers::NONE,
        Action::WorldAxes,
        "World or local axes",
        Scope::Everywhere,
    ),
    row(
        "follow.ground",
        Key::T,
        Modifiers::SHIFT,
        Action::FollowGround,
        "Follow the ground",
        Scope::Everywhere,
    ),
    row(
        "edit.delete",
        Key::Delete,
        Modifiers::NONE,
        Action::Delete,
        "Delete",
        Scope::Everywhere,
    ),
    row(
        "edit.duplicate",
        Key::D,
        Modifiers::CTRL,
        Action::Duplicate,
        "Duplicate",
        Scope::Everywhere,
    ),
    row(
        "edit.undo",
        Key::Z,
        Modifiers::CTRL,
        Action::Undo,
        "Undo",
        Scope::Everywhere,
    ),
    row(
        "edit.redo",
        Key::Z,
        Modifiers::CTRL.plus(Modifiers::SHIFT),
        Action::Redo,
        "Redo",
        Scope::Everywhere,
    ),
    row(
        "search.items",
        Key::F,
        Modifiers::CTRL,
        Action::SearchItems,
        "Search items",
        Scope::Everywhere,
    ),
    row(
        "brush.smaller",
        Key::Comma,
        Modifiers::NONE,
        Action::BrushSmaller,
        "Brush smaller",
        Scope::Everywhere,
    ),
    row(
        "brush.larger",
        Key::Period,
        Modifiers::NONE,
        Action::BrushLarger,
        "Brush larger",
        Scope::Everywhere,
    ),
    row(
        "brush.weaker",
        Key::Comma,
        Modifiers::CTRL,
        Action::BrushWeaker,
        "Brush weaker",
        Scope::Everywhere,
    ),
    row(
        "brush.stronger",
        Key::Period,
        Modifiers::CTRL,
        Action::BrushStronger,
        "Brush stronger",
        Scope::Everywhere,
    ),
    row(
        "finish",
        Key::Enter,
        Modifiers::NONE,
        Action::Finish,
        "Finish a line or area",
        Scope::Everywhere,
    ),
    row(
        "cancel",
        Key::Escape,
        Modifiers::NONE,
        Action::Cancel,
        "Cancel and go back to Select",
        Scope::Everywhere,
    ),
    row(
        "regenerate",
        Key::F5,
        Modifiers::NONE,
        Action::Regenerate,
        "Regenerate the active level in the changed area",
        Scope::Everywhere,
    ),
    row(
        "build",
        Key::B,
        Modifiers::CTRL,
        Action::Build,
        "Build",
        Scope::Everywhere,
    ),
    row(
        "problem.next",
        Key::F8,
        Modifiers::NONE,
        Action::NextProblem,
        "Next problem",
        Scope::Everywhere,
    ),
    row(
        "problem.previous",
        Key::F8,
        Modifiers::SHIFT,
        Action::PreviousProblem,
        "Previous problem",
        Scope::Everywhere,
    ),
    row(
        "help",
        Key::F1,
        Modifiers::NONE,
        Action::Help,
        "Show all shortcuts for this level",
        Scope::Everywhere,
    ),
    row(
        "settings",
        Key::F2,
        Modifiers::NONE,
        Action::Settings,
        "Open the settings",
        Scope::Everywhere,
    ),
];

impl Scope {
    fn applies(self, level: Level) -> bool {
        match self {
            Scope::Everywhere => true,
            Scope::Level(l) => l == level,
        }
    }
}

/// Whether the action can be done today. An action that cannot is listed nowhere and its key does nothing.
#[must_use]
pub fn action_exists(action: Action, app: &NavesApp) -> bool {
    match action {
        Action::Level(level) => app.levels.iter().any(|d| d.level == level),
        Action::SwitchView => app.views.can_switch(),
        Action::FrameSelection => app.selection.is_some() && ready(app),
        Action::FrameMap => ready(app),
        Action::NextProblem
        | Action::PreviousProblem
        | Action::Cancel
        | Action::Build
        | Action::Help
        | Action::HideLevel
        | Action::ShowOthersInFull
        | Action::Settings => true,
        // built by the tasks and plans that bring the tools, selection, layers, problems and settings
        Action::Tool(tool) => app.has_tool(tool),
        Action::SearchItems => app.has_palette(),
        Action::WorldAxes
        | Action::FollowGround
        | Action::Delete
        | Action::Duplicate
        | Action::Undo
        | Action::Redo
        | Action::BrushSmaller
        | Action::BrushLarger
        | Action::BrushWeaker
        | Action::BrushStronger
        | Action::Finish
        | Action::Regenerate => false,
    }
}

fn ready(app: &NavesApp) -> bool {
    matches!(app.heightmap, super::worker::HeightmapState::Ready(_))
}

/// Whether a key still fires while a text field has focus: F-keys, Esc, Enter, and Ctrl+Tab and Ctrl+B.
fn fires_while_typing(binding: Binding) -> bool {
    let ctrl = binding.modifiers.ctrl || binding.modifiers.command;
    matches!(
        binding.key,
        Key::F1
            | Key::F2
            | Key::F3
            | Key::F4
            | Key::F5
            | Key::F6
            | Key::F7
            | Key::F8
            | Key::F9
            | Key::F10
            | Key::F11
            | Key::F12
            | Key::Escape
            | Key::Enter
    ) || (ctrl && matches!(binding.key, Key::Tab | Key::B))
}

/// The keys in force: the defaults of [`SHORTCUTS`] with what the person changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Bindings {
    /// By shortcut id; `None` is "no key".
    current: BTreeMap<&'static str, Option<Binding>>,
}

impl Default for Bindings {
    fn default() -> Self {
        Self::new(&BTreeMap::new()).0
    }
}

impl Bindings {
    /// The defaults with the `overrides` of the settings file (id to key text, `""` for no key). The second value
    /// says, in words, what could not be read; such a key keeps its default. Ids this version does not know are
    /// ignored here (the settings keep them).
    #[must_use]
    pub fn new(overrides: &BTreeMap<String, String>) -> (Bindings, Vec<String>) {
        let mut notes = Vec::new();
        let current = SHORTCUTS
            .iter()
            .map(|s| {
                let binding = match overrides.get(s.id) {
                    None => Some(s.binding),
                    Some(text) if text.trim().is_empty() => None,
                    Some(text) => Binding::parse(text).or_else(|| {
                        notes.push(format!(
                            "\"{text}\" is not a key I know for {}; the default {} stays.",
                            s.label, s.binding
                        ));
                        Some(s.binding)
                    }),
                };
                (s.id, binding)
            })
            .collect();
        (Bindings { current }, notes)
    }

    fn get(&self, shortcut: &Shortcut) -> Option<Binding> {
        self.current.get(shortcut.id).copied().flatten()
    }

    /// The key of the shortcut with this id, or `None` for no key.
    #[must_use]
    pub fn key_of(&self, id: &str) -> Option<Binding> {
        self.current.get(id).copied().flatten()
    }

    /// The key that does `action` in `level`.
    #[must_use]
    pub fn key_for(&self, action: Action, level: Level) -> Option<Binding> {
        SHORTCUTS
            .iter()
            .filter(|s| s.action == action && s.scope.applies(level))
            .find_map(|s| self.get(s))
    }

    /// The key that does `action` in `level`, as text.
    #[must_use]
    pub fn key_text(&self, action: Action, level: Level) -> Option<String> {
        self.key_for(action, level).map(|b| b.to_string())
    }

    /// Whether the shortcut differs from its default.
    #[must_use]
    pub fn is_changed(&self, shortcut: &Shortcut) -> bool {
        self.get(shortcut) != Some(shortcut.binding)
    }

    /// The shortcut that already uses `new` where the shortcut `id` would use it: the same key and a scope in
    /// common (a key used everywhere meets all levels; the same key in two levels is no conflict).
    #[must_use]
    pub fn conflict(&self, new: Binding, id: &str) -> Option<&'static Shortcut> {
        let own = SHORTCUTS.iter().find(|s| s.id == id)?;
        SHORTCUTS
            .iter()
            .filter(|s| s.id != id && scopes_meet(s.scope, own.scope))
            .find(|s| self.get(s) == Some(new))
    }

    /// Gives the shortcut `id` a key.
    pub fn set(&mut self, id: &str, binding: Binding) {
        if let Some(slot) = self.slot(id) {
            *slot = Some(binding);
        }
    }

    /// Takes the key away from the shortcut `id`.
    pub fn clear(&mut self, id: &str) {
        if let Some(slot) = self.slot(id) {
            *slot = None;
        }
    }

    /// Gives the shortcut `id` its default again.
    pub fn reset(&mut self, id: &str) {
        let default = SHORTCUTS.iter().find(|s| s.id == id).map(|s| s.binding);
        if let Some(slot) = self.slot(id) {
            *slot = default;
        }
    }

    /// All defaults again.
    pub fn reset_all(&mut self) {
        *self = Bindings::default();
    }

    fn slot(&mut self, id: &str) -> Option<&mut Option<Binding>> {
        self.current
            .iter_mut()
            .find(|(k, _)| **k == id)
            .map(|(_, v)| v)
    }

    /// What differs from the default, for the settings file: the id and the key as text, `""` for no key.
    #[must_use]
    pub fn overrides(&self) -> BTreeMap<String, String> {
        SHORTCUTS
            .iter()
            .filter(|s| self.is_changed(s))
            .map(|s| {
                (
                    s.id.to_string(),
                    self.get(s).map(|b| b.to_string()).unwrap_or_default(),
                )
            })
            .collect()
    }

    /// Keys that two shortcuts share, in words. The first in the table wins when the key is pressed.
    #[must_use]
    pub fn duplicates(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, a) in SHORTCUTS.iter().enumerate() {
            for b in SHORTCUTS.iter().skip(i + 1) {
                if let Some(key) = self.get(a)
                    && self.get(b) == Some(key)
                    && scopes_meet(a.scope, b.scope)
                {
                    out.push(format!(
                        "{key} is used for {} and for {}.",
                        a.label, b.label
                    ));
                }
            }
        }
        out
    }
}

fn scopes_meet(a: Scope, b: Scope) -> bool {
    match (a, b) {
        (Scope::Everywhere, _) | (_, Scope::Everywhere) => true,
        (Scope::Level(x), Scope::Level(y)) => x == y,
    }
}

/// Whether the key went down in this frame. A key that is held down repeats; the repeats are not presses, or
/// holding H would hide and show the level over and over.
fn pressed_once(input: &InputState, key: Key) -> bool {
    input.events.iter().any(|e| {
        matches!(
            e,
            egui::Event::Key { key: k, pressed: true, repeat: false, .. } if *k == key
        )
    })
}

/// The action whose key was pressed in this frame, in the level `level`. Modifiers must match exactly, so that
/// Ctrl+B is not B. While `typing`, only keys that cannot be typed fire.
#[must_use]
pub fn action_for(
    input: &InputState,
    bindings: &Bindings,
    level: Level,
    typing: bool,
) -> Option<Action> {
    SHORTCUTS
        .iter()
        .filter(|s| s.scope.applies(level))
        .filter_map(|s| bindings.get(s).map(|b| (s, b)))
        .filter(|(_, b)| !typing || fires_while_typing(*b))
        .find(|(_, b)| pressed_once(input, b.key) && input.modifiers.matches_exact(b.modifiers))
        .map(|(s, _)| s.action)
}

/// Whether a text field has the keyboard. A `DragValue` being typed in counts, since it is one.
#[must_use]
pub fn typing(ctx: &egui::Context) -> bool {
    ctx.text_edit_focused()
}

/// The rows of the F1 sheet: what exists in `level`, everywhere first, with the key in force.
#[must_use]
pub fn sheet(app: &NavesApp, level: Level) -> Vec<(&'static str, String)> {
    let exists = |s: &&Shortcut| s.scope.applies(level) && action_exists(s.action, app);
    let everywhere = SHORTCUTS
        .iter()
        .filter(|s| s.scope == Scope::Everywhere)
        .filter(exists);
    let own = SHORTCUTS
        .iter()
        .filter(|s| s.scope != Scope::Everywhere)
        .filter(exists);
    everywhere
        .chain(own)
        .map(|s| {
            let key = app
                .bindings
                .key_of(s.id)
                .map_or_else(|| "no key".to_string(), |b| b.to_string());
            (s.label, key)
        })
        .collect()
}

/// Whether the action is built at all, whatever the state of the window: the settings list these.
#[must_use]
pub fn action_built(action: Action, app: &NavesApp) -> bool {
    match action {
        Action::Level(level) => app.levels.iter().any(|d| d.level == level),
        Action::Tool(tool) => app.levels.iter().any(|d| {
            d.tools.iter().any(|t| t.tool == tool) || (tool == ToolId::SELECT && d.pick.is_some())
        }),
        Action::SwitchView => app.views.can_switch(),
        Action::SearchItems => app.levels.iter().any(|d| !(d.items)(app).is_empty()),
        Action::HideLevel
        | Action::ShowOthersInFull
        | Action::FrameSelection
        | Action::FrameMap
        | Action::Cancel
        | Action::Build
        | Action::NextProblem
        | Action::PreviousProblem
        | Action::Help
        | Action::Settings => true,
        Action::WorldAxes
        | Action::FollowGround
        | Action::Delete
        | Action::Duplicate
        | Action::Undo
        | Action::Redo
        | Action::BrushSmaller
        | Action::BrushLarger
        | Action::BrushWeaker
        | Action::BrushStronger
        | Action::Finish
        | Action::Regenerate => false,
    }
}
