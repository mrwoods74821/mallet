# Mallet architecture

Mallet is a Hammer-style editor for Source engine VMF maps, written in Rust. It aims to
match Hammer's viewports, tools and default key bindings, and to let AI agents work on
maps through a built-in MCP server.

## Crates

| Crate | Purpose |
| --- | --- |
| `mallet-kv` | Reads and writes Valve KeyValues text. Keeps every key, block and duplicate in order and writes Hammer's exact layout (tabs, CRLF). |
| `mallet-vmf` | Typed VMF model on top of `mallet-kv`, brush geometry, and the edit/undo layer. No UI dependencies. |
| `mallet` | The editor application: window, viewports, key bindings and the MCP server. |

## Lossless loading and saving

A VMF is parsed into `Vmf`, whose world, entities, solids and sides are typed while every
block Mallet does not understand yet (`versioninfo`, `visgroups`, `dispinfo`, `connections`,
`hidden`, cordons and so on) is kept verbatim. Saving writes the same structure back, so
opening and saving a map changes nothing except number formatting that parses back to the
same values. Files that are not valid UTF-8 are read byte-for-byte and written back the same
way.

## Edits and undo

Every change to a map is an `Edit` (`mallet_vmf::edit`). Applying an edit returns its
inverse, and `History` keeps undo and redo stacks of those inverses. The keyboard, menus and
MCP tools all go through this one path, so an edit made by an agent can be undone with
Ctrl+Z and the other way round. New editing features add `Edit` variants rather than
mutating the map directly.

## Shared editor state

`Editor` (in `crates/mallet/src/editor.rs`) owns the open document, the wireframe scene,
the camera, the 2D views and the grid. It sits behind a mutex shared by:

- the window, which locks it once per frame to handle input and draw;
- the MCP server thread, which locks it once per tool call.

Tool calls therefore never wait for a frame. They keep working while the window is covered
or minimized, because Windows sends no repaints to a minimized window.

## Drawing

Viewports draw through the small `Canvas` trait. On screen it wraps an egui painter;
for screenshots it wraps a tiny-skia image. Both use the same viewport code, so a
screenshot matches what the window shows, apart from viewport labels.

The 3D view is a CPU wireframe for now. Textured rendering, lighting preview and model
display will move it to wgpu.

## MCP server

The editor starts an MCP server at `http://127.0.0.1:7414/mcp` (Streamable HTTP transport,
JSON responses). Set `MALLET_MCP_PORT` to choose another port, or to `0` or `off` to
disable it. The status bar shows the address while it runs.

Security:

- It listens on loopback only.
- Requests with a non-local `Origin` header are refused, which blocks DNS-rebinding
  attacks from web pages.
- There is no authentication, so any program on this machine can use it while Mallet runs.

Tools:

| Tool | What it does |
| --- | --- |
| `get_status` | Open map, counts, bounds, grid, camera, 2D views, undo/redo state |
| `open_map`, `save_map` | Load a VMF; save in place or to a new path |
| `list_entities`, `get_entity` | Browse entities, filtered by classname (`trigger_*` prefix match) |
| `set_entity_key`, `remove_entity_key` | Undoable key edits |
| `undo`, `redo` | Step the shared history |
| `set_grid`, `center_views` | Grid size and framing |
| `set_camera`, `set_view_2d` | Aim the 3D camera, pan and zoom the 2D views |
| `screenshot` | PNG of one or all viewports, rendered offscreen |

The repository's `.mcp.json` registers the server for MCP clients such as Claude Code.
Mallet has to be running for the tools to connect.

Adding a tool: describe it in `tools::definitions`, handle it in `tools::run_checked`, and
make any map change through `Editor::apply_edit` so it stays undoable.

## Roadmap

1. Selection, the block tool, clone, and texture application, all as `Edit`s with
   matching MCP tools.
2. Game configuration, VPK mounting, VMT/VTF materials and MDL models in the 3D view (wgpu).
3. Clip, vertex and displacement tools.
4. Lighting: a real-time preview first, then a bake through VRAD with lightmaps read back.
5. Entity I/O editor, instances, VisGroups and Check Problems.
