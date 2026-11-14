// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

// SPDX-License-Identifier: AGPL-3.0-or-later

//! The command palette: one key, then words, then Enter. It lists what the shortcuts do —
//! the levels, the level's tools and the build — and picking an entry does exactly what its
//! shortcut does.

use eframe::egui;
use naves_ui::{CmdRow, CommandPaletteProvider, MatchGroup, MatchedCmd};
use naves_ui::{FuzzyMatch, FuzzyQuery};

use super::NavesApp;
use super::levels::Level;
use super::shortcuts::{Action, ToolId};

/// What an entry does. Every one is the same as pressing its shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Go to a level.
    Level(Level),
    /// Pick a tool.
    Tool(ToolId),
    /// Build the project.
    Build,
}

impl Command {
    /// The shortcut id whose key is shown at the right of the row.
    fn shortcut_id(&self) -> Option<&'static str> {
        match self {
            // The level's own id is the shortcut id ("level.terrain").
            Command::Level(level) => Some(level.id()),
            Command::Tool(tool) => super::shortcuts::SHORTCUTS
                .iter()
                .find(|s| s.action == Action::Tool(*tool))
                .map(|s| s.id),
            Command::Build => Some("build"),
        }
    }
}

/// One row of the palette: what it does, what it says, and the key that does it too.
struct Entry {
    command: Command,
    text: String,
    kb_shortcut: String,
}

/// Offers the commands that exist right now; owned by the frame that opens the palette.
pub struct Provider {
    entries: Vec<Entry>,
}

impl Provider {
    /// The levels that are built, the active level's tools, and the build.
    pub fn new(app: &NavesApp) -> Self {
        let key_of = |command: &Command| {
            command
                .shortcut_id()
                .and_then(|id| app.bindings.key_of(id))
                .map_or(String::new(), |k| k.to_string())
        };
        let mut entries: Vec<Entry> = app
            .levels
            .iter()
            .map(|def| {
                let command = Command::Level(def.level);
                Entry {
                    command: command.clone(),
                    text: format!("Go to {}", def.level.name()),
                    kb_shortcut: key_of(&command),
                }
            })
            .collect();
        for (tool, word) in app.rail_tools() {
            let command = Command::Tool(tool);
            entries.push(Entry {
                command: command.clone(),
                text: word.to_string(),
                kb_shortcut: key_of(&command),
            });
        }
        let command = Command::Build;
        entries.push(Entry {
            command: command.clone(),
            text: "Build the project".to_string(),
            kb_shortcut: key_of(&command),
        });
        Self { entries }
    }
}

impl CommandPaletteProvider<Command> for Provider {
    fn search_hint(&self) -> &'static str {
        "A level, a tool, or build"
    }

    fn all_matching(&mut self, query: &FuzzyQuery) -> Vec<MatchGroup<Command>> {
        let group = self
            .entries
            .iter()
            .filter_map(|entry| {
                let matched = if query.is_empty() {
                    Some(FuzzyMatch::lowest(entry.text.clone()))
                } else {
                    query.try_match(entry.text.clone())
                }?;
                Some(MatchedCmd {
                    command: entry.command.clone(),
                    fuzzy_match: matched,
                    enabled: true,
                })
            })
            .collect();
        vec![group]
    }

    fn cmd_row(&self, ui: &egui::Ui, matched: &MatchedCmd<Command>, selected: bool) -> CmdRow {
        let text_color = if selected {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().widgets.inactive.fg_stroke.color
        };
        let job = egui::text::LayoutJob::simple(
            matched.fuzzy_match.target().to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            text_color,
            f32::INFINITY,
        );
        CmdRow {
            job: matched
                .fuzzy_match
                .highlight_matching_text(ui.style(), &job, selected),
            kb_shortcut: self
                .entries
                .iter()
                .find(|e| e.command == matched.command)
                .map_or(String::new(), |e| e.kb_shortcut.clone()),
            tooltip: None,
        }
    }
}

/// Runs the picked command: the same thing its shortcut does.
pub fn run(app: &mut NavesApp, command: &Command) {
    let action = match *command {
        Command::Level(level) => Action::Level(level),
        Command::Tool(tool) => Action::Tool(tool),
        Command::Build => Action::Build,
    };
    app.run_action(action);
}
