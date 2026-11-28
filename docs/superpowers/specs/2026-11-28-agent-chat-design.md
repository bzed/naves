<!--
SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Náves: talking to an agent in the editor (ACP client, chat, the editor's MCP server)

Date: 2026-11-28. Status: draft for approval. Extends `2026-11-04-naves-design.md` (5.10 MCP server),
`2026-11-06-naves-editor-ui.md` (dock, settings, shortcuts) and `2026-11-27-editing-design.md` (5.7 and phase 5);
where this document and those disagree, this one wins. UI parts made with the `frontend-design` skill.

Marks: **[decided]** the maintainer said so; **[my call]** I chose it, correct me; **[verify]** check when building.

## 1. Intent

The map maker wants to say what they want in words, while looking at the map, and have an agent do it in the open
editor. The maintainer's examples (2026-11-28):

- "in the shown forest area, change the forest to match the original location. search the web for references"
- "osm data shows a large school here, build one with the school and office models dayz provides. add the (Lenin?)
  statue in the middle of the place, also add a well"

Both speak about what the editor **shows** ("the shown area", "here") and both **change the map**. Today neither is
possible: `naves mcp <dir>` is a separate process that does not know the running editor, and it has no edit tools.

Maintainer's decisions (2026-11-28):

- Náves integrates the **Agent Client Protocol (ACP)** and gets a **chat window**. **[decided]**
- The MCP server connection and similar details are **passed to the agent automatically**. **[decided]**
- The GUI needs an **MCP connection that can edit things**. **[decided]**
- This design covers **the basic integration only**. Editing forests, placing models and every other kind of edit
  gets its MCP tools when that feature is built. **[decided]**

A note on the roles, because the request said "connect to the ACP server": in ACP the editor is the **client** and
the agent (Claude Code, Gemini CLI, Codex, Mistral Vibe, OpenCode, …) is a **subprocess** that speaks JSON-RPC on its
standard input and output. Náves starts the agent. The MCP servers the agent should use are part of the
`session/new` request, and that is where "passed automatically" happens.

## 2. Goals, non-goals, anti-goals

**Goals**

1. A chat in the editor: pick an agent once in the settings, type, read the answer as it streams, stop it.
2. The agent works **in the running editor**: it can ask what is shown and selected, move the view, and its
   changes are drawn at once, saved like a hand-made edit, and undone with Ctrl+Z.
3. No hand configuration: Náves starts the agent in the project folder and hands it the editor's MCP server.
4. One tool table for both ways of running the MCP server (in the editor, and `naves mcp <dir>` without it).
5. Adding an edit tool for a later feature (terrain, placed models, forest) is one more entry in that table plus
   one more request the editor answers. Nothing in the chat or the ACP client changes for it.
6. The first edit tools: roads and paths, the only edit operations that exist today.

**Non-goals (this design)**

- Editing forests, terrain, buildings and objects, and a search over the game's models. They come with their
  features (editing design, phases 3 and 4, and the vegetation work). The two example sentences of section 1 work
  when those tools exist, not at the end of this design.
- The approval mode of 5.10 (proposals drawn on the map, an "AI proposals" tab). This design has the auto mode
  only, plus the permission questions an agent asks itself (5.5). **[my call]**
- Keeping a conversation across a restart of the editor (`session/load`), several chats at once, slash commands,
  agent modes, attaching files or pictures to a message.
- Offering the agent ACP's file and terminal services. Náves declares neither; an agent uses its own tools.
- Installing agents, or a registry of them. The user installs an agent and gives its command.
- An LLM client inside Náves. Náves never talks to a model provider and stores no API key.
- A network port. Nothing listens on TCP.

**Anti-goals** (would make it a failure even if it works)

- A change by the agent that the editor does not show, or that Ctrl+Z does not take back.
- A second code path for edits: the agent must go through `naves_core::edits` and the editor's edit session, as
  the tools on the rail do.
- Anything of the chat or the agent in the build output: builds stay reproducible from the project folder.
- An MCP endpoint another local user can reach.
- A frozen editor while the agent or a tool is busy.
- A chat that covers the map.

## 3. Constraints

- AGENTS.md in full. In particular: everything the agent and the MCP client send is **untrusted input** (no panics,
  limits on sizes and counts, a fuzz target for what we parse ourselves); processes are started with an argument
  list, never a shell; a missing agent program is found up front and named; `tracing` in libraries; test first with
  fakes; coverage at or above 85 %; the manual, the CLI help and `CHANGELOG.md` change in the same commits.
- Dependencies are reviewed before use and the review goes into `docs/research/2026-11-04-dependency-reviews.md`.
- Linux first; everything also builds and works on Windows.
- GUI chrome: dark by default, colourblind-safe, no bright brownish colours (maintainer, 2026-11-27). Defaults for
  keys on the left half of the keyboard (maintainer, 2026-11-28).
- No secrets in logs, error messages, the repo or the settings file.
- The async rules: no `std::sync::Mutex` across `.await`, heavy work in `spawn_blocking`.

## 4. Approaches

The open choice is **how the agent reaches the MCP server that lives inside the editor**. The agent is a child of
the editor, and it starts its MCP servers itself from the list in `session/new`.

**A. A stdio bridge to a local socket (picked).** The editor listens on a Unix domain socket (Windows: a named
pipe). The list in `session/new` has one stdio server: the Náves binary itself, `naves mcp --attach <endpoint>`,
which copies bytes between its standard streams and the socket. Every ACP agent must support stdio servers, so this
works with all of them. No TCP port, and no new dependency for it (tokio has both kinds of endpoint; rmcp serves any
pair of async streams). Cost: one small extra process per MCP connection, and endpoint code for two platforms.

**B. MCP over HTTP on 127.0.0.1 with a token.** The editor serves streamable HTTP. No bridge process. But an agent
supports HTTP servers only if it says so in its capabilities, it opens a TCP port, and it switches on rmcp's HTTP
stack (a web server in the dependency tree), which the dependency review of rmcp explicitly kept off.

**C. MCP over ACP.** The MCP messages travel inside the ACP connection that already exists; no socket, no bridge.
The cleanest design, and the `agent-client-protocol` crate has it, but behind `unstable_mcp_over_acp`, and the
agent must support it too. Not usable as the only way today.

Picked: **A**, because it is the only one that works with every agent now and it keeps Náves off the network. The
maintainer handed me the how, so this is **[my call]**. The seam in 5.2 (a transport is a pair of byte streams)
lets C be added later as a second transport without touching the tools.

Two smaller choices, both **[my call]**:

- **Where the server's tools get their data.** Rejected: a second copy of the project in the server (today's
  `NavesMcp` caches) next to the editor's state; the two would disagree. Picked: a host seam (5.1). In the editor
  the tools ask the editor; standalone they do what they do today.
- **Where the chat is.** Rejected: a floating window (the UI spec rejects floating panels: they cover the map); a
  new side panel (the map loses width for good). Picked: a tab of the bottom dock (5.6), where the UI spec already
  puts History and AI proposals.

## 5. Design

```
 editor process                                            child processes
+--------------------------------------------------+
| GUI thread (egui)                                |
|   NavesApp: project, view, selection, edit       |
|   session, workers                               |
|        ^  requests, answered at frame start      |
|        |                                         |
| agent runtime (one tokio runtime, own threads)   |
|   +-- ACP client -------- stdin/stdout ----------+--> agent (claude, gemini, ...)
|   |     events -> chat                           |        |
|   +-- MCP server, editor host                    |        | starts, from session/new
|         ^                                        |        v
|         +-------- local socket ------------------+--- naves mcp --attach <endpoint>
+--------------------------------------------------+
```

### 5.1 One MCP server, two hosts

`crates/naves/src/mcp.rs` becomes a module with the tool table and a **host** behind it:

- `Host` is the seam: the project, the map data for rendering (heightmap, surfaces, imagery, generated roads),
  setting a project field, running a build, the edit operations, and the view. An object-safe async trait or an
  enum of the two hosts; the builder picks.
- **`StandaloneHost`** is today's behaviour, moved: it reads the project folder, keeps its caches, writes
  `naves.toml`. `naves mcp <dir>` uses it. Its results stay as they are; `crates/naves/tests/mcp.rs` must pass
  unchanged, apart from the new tools appearing in the tool list.
- **`EditorHost`** holds a channel to the GUI thread and the `egui::Context` (to wake it with `request_repaint`).
  Each tool call becomes a request with a one-shot reply.

How the editor answers:

- The GUI drains the requests at the start of a frame, each within a small time budget, and never blocks on one.
- **Light requests** (view, selection, an edit, a project field) are answered in that frame from `NavesApp`.
- **Heavy reads** (`render_topdown`, `roads_stats`, `validation_list`, `heightmap_stats`): the GUI answers with the
  `Arc`s it already holds (project, heightmap, roads, imagery); the rendering and counting run in the runtime's
  blocking pool. If the editor is still computing what is asked for, the tool waits for it, with a deadline, and
  then says "The roads are still being generated, try again".
- **`project_set`** goes through the same path as a change in the settings panel, so the panel shows the value and
  the heightmap reloads.
- **`build`** starts the editor's build worker, as the Build tab does, and returns when it ends. A build that is
  already running is an error that says so.
- **`imagery_fetch`** runs in the blocking pool; when it ends the editor reads the project again.
- A request the editor cannot answer because no project is open, or because the window closed, is a tool error, not
  a hang: every wait has a deadline.

### 5.2 The endpoint and the bridge

- The editor creates the endpoint when a project is open and "Let agents use the editor" is on (5.7):
  - Unix: a socket with a random name in a new directory with mode 0700, under `$XDG_RUNTIME_DIR` when it is set
    and otherwise under the temporary directory (`tempfile`), removed when the editor closes;
  - Windows: a named pipe with a random name that rejects remote clients.
- A random token (128 bits or more, from the OS) is made per editor run. The first line a client sends must be the
  token; anything else closes the connection. The token is compared in constant time, and neither it nor the
  endpoint name is logged.
- `naves mcp --attach <endpoint>` reads the token from the environment variable `NAVES_MCP_TOKEN`, connects, sends
  it, and then copies bytes both ways until either side closes. It holds no project state. If it cannot connect it
  exits with a message that says the editor is not running. `naves mcp <dir>` is unchanged; `--attach` and a
  directory together are an error.
- Several connections at once are allowed (an agent may start the server again for a sub-agent). All share the one
  editor.
- The server code takes "a pair of byte streams", so the tests use an in-memory pair, and MCP over ACP (approach C)
  can be a second transport later.

### 5.3 Tools

All existing tools stay, with the same names, arguments and results. New, in **both** hosts unless noted:

| Tool | Arguments | Result |
|---|---|---|
| `view_get` (editor only) | none | `{rect: {x0, y0, x1, y1}, level, base_map, layers, tool, selection, pointer}`: what the editor shows, in map metres; `selection` and `pointer` are `null` when there is none |
| `view_go` (editor only) | `x0, y0, x1, y1`, or `x, y` | moves the map view there; returns the new `view_get` |
| `roads_list` | `x0, y0, x1, y1`, `limit`? | `{ways, more}`: the ways that touch the rectangle, edits applied, each with `id`, `origin` (`osm`, `edited`, `new`), `tags` and `points` (map metres) |
| `road_add` | `points`, `tags` | adds a way; returns it with its new `id` |
| `road_modify` | `id`, `points`?, `tags`? | replaces the line, the tags, or both; returns the way |
| `road_delete` | `id` | deletes the way |
| `road_reset` | `id` | drops the edit of this way, so that the OpenStreetMap original is back |

Rules for these tools:

- **Coordinates** are map metres from the south-west corner, as `render_topdown` already has them. The tools
  convert to and from latitude and longitude with the project's projection.
- **The edit tools use `naves_core::edits`** (`add_way`, `modify_way`, `move_node`, `new_node`, `delete_way`,
  `reset_way`) with `By::Ai`. How a new list of points maps onto moved, added and removed nodes is the builder's;
  junction nodes shared with other ways must stay shared when their point did not move.
- **In the editor**, a tool call is one `EditSession::apply`: saved atomically, drawn at once, one undo step, and
  the status line says "Edited: press Ctrl+R to rebuild" as after a hand-made edit. The agent's edits and the
  user's edits are in the same undo history, in the order they happened.
- **Standalone**, the same operation is applied to the edit files in the project folder and written atomically.
  There is no undo there.
- **Limits:** at most 10 000 points per way, 64 tags, 255 bytes per key and value, 1 000 ways per `roads_list`
  answer (with `more: true` when cut), finite coordinates inside the map. A breach is a tool error that names the
  limit. **[my call]** on the numbers; they go to `naves_core::limits` where the edit files' limits are.
- An edit that the edit API refuses is a tool error with its message ("The edit cannot be made: …"), and nothing
  changes.
- `view_get` and `view_go` are not in the tool list of the standalone server.
- **Server instructions:** the server's `instructions` (MCP `initialize`) explain, in a few sentences, the
  coordinates, that `view_get` says what "here" and "shown" mean, that `render_topdown` is how to look, that edits
  apply at once and the user can undo them, and that a build is needed to see edits in the generated roads. This is
  the only "system prompt" Náves gives, so every agent gets it, in the editor or not.

A later feature adds its tools here (for example `forest_*`, `model_place`, `catalog_search`) with a `Host` method
each; phase 5 of the editing design ("MCP edit tools") is replaced by "each phase brings its MCP tools".

### 5.4 The ACP client

A module of the `naves` binary (`crates/naves/src/agent/`), with no egui in it. It runs on the agent runtime and
talks to the GUI through two channels: commands in (start, prompt, answer a permission question, cancel, stop) and
events out (text, thoughts, tool calls, plans, questions, state, errors).

- **Crate:** `agent-client-protocol` (5.6.1, Apache-2.0, the protocol's own Rust SDK, needs Rust 1.88), stable
  protocol **v1** only, no `unstable_*` feature. It is runtime-neutral (`futures`); its `process` feature starts
  the child with `async-process`. **[verify]** in the dependency review: whether to use `process` or to start the
  child with `tokio::process` and hand the crate the streams; the review of AGENTS.md is still to do and may reject
  the crate, in which case the question comes back to the maintainer.
- **Start:** the command and arguments come from the settings (5.7). The program is looked up before starting; if
  it is missing, the error names it and the setting. The child gets the project folder as working directory and the
  editor's environment, unchanged. Its standard error is kept (the last 64 KiB) for error messages and is not
  logged above `debug`.
- **`initialize`:** protocol version 1, client name and version, client capabilities: **none** (no `fs`, no
  `terminal`). A different protocol version in the answer is an error that says the agent is too old or too new.
- **Login:** if the agent answers `session/new` with "authentication required", the chat shows the agent's
  authentication methods as buttons and calls `authenticate` with the chosen one. If that fails, the chat says to
  log in with the agent's own program first, and shows the agent's message.
- **`session/new`:** `cwd` is the project folder. `mcpServers` has one entry when the endpoint exists:

  ```json
  {
    "name": "naves",
    "command": "/full/path/of/the/running/naves",
    "args": ["mcp", "--attach", "<endpoint>"],
    "env": [{ "name": "NAVES_MCP_TOKEN", "value": "<token>" }]
  }
  ```

  The command is `std::env::current_exe()`, so that the agent gets the version of Náves that is running. Without
  the endpoint (the setting is off) the list is empty and the chat says once that the agent cannot see the map.
- **A prompt** is the user's text and, in front of it, one text block that Náves writes: the project's name, the
  view rectangle, the active level, the selection and the pointer position, each on a line. So "here" and "the
  shown area" mean something without a tool call. The chat shows the user's text only; the block can be unfolded
  under it.
- **Updates** (`session/update`) become events: message text, thoughts, tool calls with their status and content,
  plans. Unknown kinds are ignored. Text and tool output are capped per message (1 MiB) and per conversation
  (4 000 entries, the oldest dropped) so that a runaway agent cannot fill the memory.
- **`session/request_permission`** becomes a question in the chat (5.6); the answer is the option the user picked,
  or "cancelled" when the turn was stopped.
- **Stop:** `session/cancel`; the turn ends when the agent says so. If it does not within 5 s the chat offers to
  restart the agent.
- **End:** closing the project or the editor, switching the agent, or "New chat" ends the session; the child's
  standard input is closed, and it is killed if it has not left after 3 s. "New chat" starts a new session in the
  same child when it is alive.
- **The child dies:** the chat says so, with the end of its standard error, and offers "Start again". The
  conversation so far stays readable.
- **After every turn** the editor checks whether `naves.toml` or the files under `edits/` changed on disk without
  it (an agent may use its own file tools) and reads them again. A changed edit file becomes one undo step when the
  new content can be read, and a Problems row when it cannot (the editor keeps what it has). **[my call]**

### 5.5 What the agent may do

- The agent is a program the user chose, and it runs **with the user's rights**. Náves does not sandbox it. An
  agent with its own shell or file tools can do what the user can do. The manual says this in plain words, next to
  the setting.
- Through MCP the agent can do what the tools do: read, render, change settings, edit roads, build.
- **Mode:** auto only. Edits apply at once and are undoable. Whether an agent asks before using a tool is the
  agent's own setting; when it asks, the question appears in the chat with the agent's options (allow once, always
  allow, reject, …). Náves answers nothing on its own: there is no "allow everything" switch in Náves.
- The approval mode of 5.10 (proposals on the map) stays planned; it will sit in `EditorHost`, between the tool
  call and `EditSession::apply`, and needs no change in the tools.

### 5.6 The chat

A third tab in the bottom dock, **Agent**, after Build and Problems. The dock is wide and low, so the tab has two
columns: the conversation, held to a readable line length, and beside it what the agent **did**, as rows that go
to the map. Wide lines of running text would be hard to read; a list of actions uses the width well, and it is the
part that matters to someone who is watching the map.

```
+--------------------------------------------------------------------------------------------------+
| Build | Problems 7 | Agent ●                                                          Settings   |
+--------------------------------------------------+-----------------------------------------------+
| You                                              | Claude Code            New chat   Start again |
|   The road shown here is too steep. Lead it      |-----------------------------------------------|
|   along the valley instead.                      | ✓ view_get                                    |
|   ▸ sent with: view 4120,880 – 5300,1710, Roads  | ✓ render_topdown   slope, roads               |
|                                                  | ✓ roads_list       14 ways                    |
| Agent                                            | ✓ road_modify      way 8421      go to  undo  |
|   The upper part climbs 19 %. I moved it to      | … road_add         new way                    |
|   the east bank, where the slope stays under     | ? Run `curl …` in the project folder?         |
|   9 %, and I am adding the link to the bridge.   |     Allow once   Always allow   Reject        |
|                                                  |                                               |
| [ Ask about the map, or say what to change   ] ⏎ |                                               |
| Stop                                             |                                               |
+--------------------------------------------------+-----------------------------------------------+
```

- **Left column, the conversation.** At most about 70 characters wide. The user's and the agent's messages in
  order, the agent's as Markdown (`egui_commonmark`, already in the tree through `naves-ui`; **[verify]** that it
  can be used from the binary crate without a second copy). The agent's thoughts are one folded line, "Thought for
  a moment", that unfolds. The text field is at the bottom: Enter sends, Shift+Enter is a new line. While a turn
  runs, the send button is "Stop". The view follows new text unless the user scrolled up.
- **Right column, what the agent did.** One row per tool call: a mark for its state, the tool's name, a few words
  from the agent's title. Rows that changed the map have "go to" (frames the place, as a Problems row does) and the
  newest of them has "undo" (the same as Ctrl+Z, offered only while that edit is the last one). A row unfolds to
  the call's content: text, a picture (a render the agent looked at), a diff. A plan from the agent is a checklist
  at the top of the column. A permission question is a row with the agent's options as buttons; it has the keyboard
  focus when it appears, and the tab's dot turns into a question mark.
- **The header of the right column** names the agent in use (a menu when several are set up) and has "New chat"
  and, when the agent has stopped, "Start again".
- **Narrow windows:** under about 900 px of dock width the right column goes away and its rows appear in the
  conversation, between the messages, in the order they happened.
- **The tab's label** is "Agent", with a dot while a turn runs and "?" while a question waits, so that it can be
  seen from the Build or Problems tab. No sound, no animation other than the streaming text.
- **State is never colour alone:** ✓ done, … running, ✗ failed, ? waiting for you, each also said in words for
  screen readers (`WidgetInfo`), in the colours of `theme.rs` (`active`, `problem`), nothing new.
- **Words on empty and failed states:**
  - no agent set up: "No agent is set up. Choose one in Settings → Agent." with a button that opens that tab;
  - program missing: "Cannot start gemini: the program was not found. Set its path in Settings → Agent.";
  - no project open: "Open a project to talk about it.";
  - the agent stopped: "The agent stopped." and the end of what it printed, with "Start again".
- **Keys:** `Ctrl+E` opens the Agent tab and puts the cursor in the text field (left half, free in the table today;
  **[verify]** with the test for double assignments). Esc in the text field gives the keyboard back to the map;
  Esc while a turn runs does **not** stop it (too easy to hit while working on the map), "Stop" does. The action is
  in the command palette and can be rebound like every other.
- "Typing is typing": while the text field has the focus, no single-key shortcut fires (the existing rule).

The agent starts with the first message, not when the editor opens.

### 5.7 Settings

A tab **Agent** in the settings window (the UI spec planned it as "AI"), saved in the per-user settings file under
`[agent]`, like the other preferences (unknown keys kept):

```toml
[agent]
use = "claude"            # which one the chat starts
editor_access = true      # "Let agents use the editor": the MCP endpoint of 5.2

[[agent.agents]]
id = "claude"
name = "Claude Code"
command = "claude-code-acp"
args = []
```

- The tab lists the agents (name, command, arguments), with add, remove and "Test", which starts the agent, does
  `initialize`, shows its name and version or the error, and stops it.
- "Add" offers presets that fill in the fields for the common agents (Claude Code, Gemini CLI, Codex, Mistral Vibe,
  OpenCode). The commands of the presets change with the agents' releases: **[verify]** each when building, and the
  manual says where to look when one has changed. A fresh install has **no** agent set up; nothing is started that
  the user did not choose.
- No environment variables and no keys in the settings: the agent inherits the editor's environment and uses its
  own login. **[my call]**
- `editor_access = false` removes the endpoint; chats still work, without the map.
- The settings are per user, not per project: the project file gets nothing, so a shared project never starts a
  program on someone else's machine.

### 5.8 Errors and limits

- Every message from the agent and from MCP clients is parsed by the two SDKs; what we build on top (the token
  line, the tool arguments, the limits of 5.3 and 5.4) returns errors and never panics. The token line gets a fuzz
  target together with the tool-argument parsing of the new tools.
- A line from the agent longer than 16 MiB ends the session with an error (**[verify]** whether the crate can cap
  it; otherwise a capping reader in front of it).
- All waits have deadlines: connecting, `initialize` (30 s), a request to the GUI (10 s for light requests; a
  build has the build's own timeout), ending the child.
- Errors shown to the user name what failed and what to do; they never contain the token.

## 6. Tests

- **Host seam:** the existing `tests/mcp.rs` against `StandaloneHost`, unchanged. The new road tools against a
  synthetic project: add, modify, delete, reset, the limits, an edit the API refuses.
- **Editor host:** with `egui_kittest` and an rmcp client on an in-memory stream pair: `view_get` reports the view
  and selection that were set; `view_go` moves it; `road_modify` changes the edit session, writes the file, and one
  undo takes it back; an edit by the user and one by the agent undo in order; a request while no project is open is
  an error; a heavy read does not stop frames from being drawn.
- **Endpoint and bridge:** a wrong token is closed; a right one is served; `naves mcp --attach` against a test
  endpoint in a `TempDir` passes a tool call through and exits when the endpoint closes; the socket's directory has
  mode 0700 (Unix).
- **ACP client:** against a **fake agent**: in-process on a stream pair for the cases (initialize, version
  mismatch, login needed, a streamed answer, a tool call, a plan, a permission question and each answer, cancel, a
  message over the cap, garbage), and one test with a real child process (a small test program built with the
  crate) that checks `session/new` carries the working directory and the `naves` MCP entry, and that the child is
  gone after the session ends. No test starts a real agent or uses the network.
- **Chat:** kittest: the empty states, sending, streaming text, the rows, "go to" and "undo", a question gets the
  focus and its answer is sent, the narrow layout, the tab's mark, `Ctrl+E`, and that typing does not fire tool
  shortcuts.
- **Settings:** round trip of `[agent]`, unknown keys kept, a damaged section is reported like the others.
- **By hand, once per preset** (`#[ignore = "needs <agent>"]` where it can be automated): the agent starts, sees
  the tools, a road edit shows on the map.
- No `sleep` to wait for state: deadlines and polling helpers, `tokio::time::pause` where it helps.

## 7. Phases

Each phase has its own plan and ends with a working, tested state.

1. **The host seam and the road tools** (no GUI change): `Host`, `StandaloneHost`, `roads_list`, `road_add`,
   `road_modify`, `road_delete`, `road_reset`, the server instructions. Usable at once with `naves mcp <dir>`.
2. **The editor's MCP server:** `EditorHost`, the request queue in the GUI, `view_get`, `view_go`, the endpoint,
   the token, `naves mcp --attach`, the setting. Usable at once from any external MCP client that is given the
   bridge command, which is also how it is tested by hand before the chat exists.
3. **The ACP client** (no chat yet): the dependency review, the module, the fake agent, the reload after a turn.
4. **The chat and the settings tab**, the manual pages, the presets.

## 8. Documentation

- `docs/manual/src/mcp.md`: the new tools, `--attach`, the two ways of running the server.
- A new page `docs/manual/src/agent.md` (and `SUMMARY.md`): setting up an agent, the chat, what the agent can do
  and that it runs with the user's rights, troubleshooting (program not found, login, the agent cannot see the
  map).
- `gui.md`, `settings.md`, `shortcuts.md`, `cli.md` (the `--attach` option; `clap` help must agree).
- `CHANGELOG.md` under Unreleased, per phase.
- `2026-11-27-editing-design.md`: phase 5 is replaced by this design (a one-line note there).

## 9. Left to the builder

- The shape of `Host` (trait or enum) and of the request type between the server and the GUI.
- How a new point list maps onto node operations in `road_modify`.
- `process` feature of the ACP crate or `tokio::process`; whether the agent runtime is the runtime the worker
  threads could share later.
- The named-pipe details on Windows.
- The exact wording of the server instructions and of the context block in front of a prompt.
- The exact widths and the fold threshold of the Agent tab, within 5.6.

## 10. Calls I made (to check while reading)

1. Approach A, the stdio bridge to a local socket (section 4).
2. Auto mode only; the approval mode of 5.10 is later (2, 5.5).
3. Road edit tools are in both hosts, also in `naves mcp <dir>` (5.3).
4. `view_go`: the agent may move your view (5.3).
5. A context block with view and selection goes in front of every prompt (5.4).
6. Náves offers the agent no file or terminal service; agents use their own (2, 5.4).
7. The chat is a dock tab with two columns, and Esc does not stop a turn (5.6).
8. No agent is preset as the default, no environment variables or keys in the settings, settings per user only
   (5.7).
9. The conversation is lost when the editor closes (2).
10. The model search (`catalog_search`) that I first counted in is left to the placed-models phase: today's
    catalogue holds road pieces, junctions and bridges only.
