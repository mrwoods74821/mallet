# Mallet architecture

Mallet is a Hammer-style editor for Source engine VMF maps, written in Rust. It aims to
match Hammer's viewports, tools and default key bindings, and to let AI agents work on
maps through a built-in MCP server.

## Crates

| Crate | Purpose |
| --- | --- |
| `mallet-kv` | Reads and writes Valve KeyValues text. Keeps every key, block and duplicate in order and writes Hammer's exact layout (tabs, CRLF). |
| `mallet-vmf` | Typed VMF model on top of `mallet-kv`, brush geometry, and the edit/undo layer. No UI dependencies. |
| `mallet-format` | The `.mallet` map format: the `Vmf` model as compact binary, with a checksum per section and room for the editor's own state. |
| `mallet-content` | Game content: `gameinfo.txt` search paths, VPK archives, VMT materials, VTF textures, MDL models and FGD entity classes. |
| `mallet` | The editor application: window, viewports, key bindings and the MCP server. |

The icon lives in `branding/`: `make_icons.py` draws the mallet once and writes the SVG,
PNGs and the multi-size `.ico` (with a simpler drawing at 24 pixels and below). `build.rs`
embeds the `.ico` and version details in the executable on Windows; `branding.rs` gives the
window, menu bar, About window and MCP server's `serverInfo` the same picture.

## Lossless loading and saving

A VMF is parsed into `Vmf`, whose world, entities, solids and sides are typed while every
block Mallet does not understand yet (`versioninfo`, `visgroups`, `dispinfo`, `connections`,
`hidden`, cordons and so on) is kept verbatim. Saving writes the same structure back, so
opening and saving a map changes nothing except number formatting that parses back to the
same values. Files that are not valid UTF-8 are read byte-for-byte and written back the same
way.

The one thing saving adds is where the map was viewed from, as Hammer does: the 3D camera
goes into the active entry of the `cameras` block (other cameras are kept) and the grid
size into `viewsettings`. Opening a map puts the camera and grid back, in Mallet and in
Hammer. This is not an edit, so it neither dirties the map nor enters the undo history.

## The .mallet format

`mallet-format` writes the `Vmf` model straight to bytes, without going through KeyValues
text. Sides are fixed records: their id, plane points and texture axes are numbers (whole
numbers as zigzag varints of one to three bytes, anything else as the exact `f64`), and
every key, value, block name and material is an index into a string table that holds each
string once. Everything `Vmf` keeps verbatim is kept the same way, along with where a
side's extra items sat among its typed fields, so VMF to `.mallet` to VMF gives the same
bytes. The map section is LZ4 compressed.

A file is a signature (`89 'MALL' 0D 0A 1A`, which catches text-mode copies), a format
version and a list of sections, each tagged, sized and checksummed with CRC-32. `MAP ` holds
the map; `META` is JSON that `Document` fills in, currently the writing Mallet's version and
the text encoding the map had as a VMF, so exporting it again writes the same bytes.
Readers skip sections they do not know, and refuse a newer format version with a clear
message.

`Document::load` tells the formats apart by the signature, and `Document::save` and
`Document::export` choose by the extension. On City 17 (28.5 MB of VMF, 9,289 brushes and
9,504 entities) a release build reads the map in 119 ms against 328 ms for VMF and writes
it in 140 ms against 554 ms, and the file is 2.8 MB. `MALLET_BIG_VMF=<map> cargo test
--release -p mallet-format big -- --nocapture` measures it on any map.

The compilers read VMF only. For a `.mallet` map, `Editor::compile_context` points the
compile at `mallet_compile/<name>.vmf` beside the map (or the folder Mallet Options names), and `start_compile` writes that copy
of the map as it is, saved or not, before the first step.

`shell.rs` sets up the Windows file types from Mallet Options > Files, for the current
user only (`HKEY_CURRENT_USER\Software\Classes`, no administrator rights): `.mallet` points
at the `Mallet.Map` program id, and `Mallet.Vmf` is added to `.vmf\OpenWithProgids` so the
default program for VMF files is left alone. Both open `"<mallet.exe>" "%1"`. When Mallet
starts with a map, `handoff.rs` first offers it to a Mallet already listening on the MCP
port (`open_map` with `focus`, after `AllowSetForegroundWindow` so that copy may come to
the front) and exits if one takes it, so double-clicking maps fills tabs of one window.
Copies with their own `MALLET_MCP_PORT` stay apart.

## Edits and undo

Every change to a map is an `Edit` (`mallet_vmf::edit`). Applying an edit returns its
inverse, and `History` keeps undo and redo stacks of those inverses. The keyboard, menus and
MCP tools all go through this one path, so an edit made by an agent can be undone with
Ctrl+Z and the other way round. New editing features add `Edit` variants rather than
mutating the map directly.

Edits that add brushes or entities carry them with ids already assigned by
`IdAllocator`, which starts above every id in the map, including ids inside blocks kept
verbatim. Moving a brush shifts its texture offsets so textures stay locked to it, and
moves a displacement's start position with it. Resizing (`Edit::Rescale`) maps every plane
point from the old bounding box to the new one; its undo is `Edit::ReplaceSolids` holding
the original brushes, so undo is exact instead of scaling back with rounding errors.

## Selection

World brushes are selected one by one; clicking any part of an entity selects the whole
entity, like Hammer's default "groups" mode. The selection lives in `Editor` as a set of
`Owner`s, so the window and MCP tools share it. 2D clicks pick, like Hammer, a brush's
centre × first and then its outline, preferring the smallest brush; empty space inside a
brush picks nothing. 3D clicks cast a ray against each brush's planes, skipping a brush
the camera is inside; in mouselook the ray goes through the crosshair. Props are tested
against their model's triangles, in model space through the inverse of their placement,
so a click through a gap in a fence picks the wall behind it; only when the ray hits
nothing at all does a prop's bounding box count, so thin or hollow models can still be
picked. The texture tool uses the same test, so props block it only where they are. The camera smooths
raw mouse movement over two frames and eases its speed, using real frame times.

## Map tabs

Several maps can be open at once, one per tab. The active tab's map state (document and
undo history, scene, views, camera, selection, tool previews) lives directly in `Editor`,
so all other code only ever deals with one map; switching tabs parks that state in a
`MapState` and moves the other tab's state in. Settings, the game's materials and the GPU
renderer are shared, so textures load once for every tab. GPU caches are keyed by the
scene's generation numbers, which are unique across tabs, so a switch simply redraws.

## Window layout

The chrome around the viewports lives in `app/chrome.rs`, laid out like Hammer++: an icon
toolbar (Phosphor icons) under the menus, the tab strip, the tool palette on the left and
the object panel on the right. Keyboard shortcuts are ignored while a text field has
focus.

## Shared editor state

`Editor` (in `crates/mallet/src/editor.rs`) owns the open document, the wireframe scene,
the camera, the 2D views and the grid. It sits behind a mutex shared by:

- the window, which locks it once per frame to handle input and draw;
- the MCP server thread, which locks it once per tool call.

Tool calls therefore never wait for a frame. They keep working while the window is covered
or minimized, because Windows sends no repaints to a minimized window.

## Game content

`GameFs` mounts the `game` search paths of a game's `gameinfo.txt` in priority order:
loose folders, `folder/*` wildcards such as GMod's addons, and VPK archives (only their
directory trees are read up front). After them come the games Garry's Mod lists in
`cfg/mount.cfg`, each folder with every `*_dir.vpk` in it, unless Mallet Options turns
them off. Every mount belongs to a `Source` (a game VPK or folder, a mounted game, an
addon or downloads), and listings say which source each file is read from, so the texture
browser can filter by them. `Materials` turns a material name into the texture
Hammer shows: `%tooltexture` first, then `$basetexture`, following `patch` materials. VTF
images are decoded at the largest mip level no bigger than 512 pixels, and loading runs on
every CPU core. Materials that cannot be found render as Hammer's pink and black checkers.

`load_model` reads a studio model's `.mdl`, `.dx90.vtx` and `.vvd` through `vmdl` and keeps
the first level of detail with skin 0. Each mesh's material is resolved against the
model's `$cdmaterials` folders in order, as the engine does. The parser panics on some
features it lacks (animations in separate blocks), so a panic counts as a failed load.
The editor loads every model a map uses in parallel when the map opens and keeps them in
a cache shared by all tabs, with the error for each model that failed.

## Entity classes

`GameData` reads FGD files, following `@include`, and merges each class with its base
classes: keyvalues with types, defaults, help and choices, spawnflags, inputs and outputs,
and the `size()`, `color()`, `studio()` and `iconsprite()` helpers. Base classes have their
own namespace, since `base.fgd` has both a `Light` base class and a `light` point class.
Mallet reads the FGD in the game's `bin` folder named after the game folder (such as
`garrysmod.fgd`) unless Tools > Game Data Files picks others. The scene uses a class's
size and colour for point entities and shows its `studio()` model, such as the player
figure of `info_player_start`. New entities start with their class's defaults.

Outputs are `Connection`s read from and written to an entity's `connections` block, keeping
the separator the map uses, and change through the undoable `Edit::SetConnections`.

## Object Properties

Alt+Enter, Edit > Properties or a double-click opens one window for the selected entities
(the world for world brushes). It edits every selected entity at once and shows
"(different)" where they disagree. Tabs: Keyvalues, with an editor per FGD type
(choices, colour pickers, target names from the map) and a Raw keys mode; Outputs, with
named-target checks and pickers for the class's outputs and the target's inputs; Inputs,
listing the outputs elsewhere that name this entity; and Flags. Help for the hovered key
shows at the bottom. Text fields commit when they lose focus, and every change is an
undoable edit, so there is no Apply button.

## Texturing

`mallet_vmf::texture` holds the Face Edit math, tested on its own: Hammer's world axes
(Quake's base axes, floors first), face axes that follow slopes, rotation about the face
normal, and justify and fit, all on `u = point · axis / scale + offset`.

The texture tool keeps a set of selected side ids per tab; scene faces carry their side and
brush ids, so a ray picks one face through the plane it enters. Every face change replaces
the brushes involved with `Edit::ReplaceSolids`, which undoes exactly. A drag passes a group
to `History::apply_grouped`, which keeps only the first step's inverse, so the whole drag
undoes at once.

Texture changes do not change shape, so they skip the full scene rebuild: `Scene::retexture`
rebuilds only the meshes of the brushes involved and records the step in
`Scene::retextures`. The renderer keeps every chunk's triangles on the CPU; when it can
follow the recorded steps from what it last drew to the current generation, it
re-triangulates only the chunks those meshes are in. A face edit on City 17 (9,289 brushes)
takes about 10 ms instead of 250.

The texture browser lists materials with their sources on a thread, describes every VMT in
the background for the kind filters (`MaterialInfo`), and loads thumbnails on four worker
threads, newest request first, so what is on screen loads before what scrolled past.

## Compiling

`compile` turns a `Preset` (a list of `Step`s with options from a per-tool catalog,
extra arguments and, for custom steps, a program, arguments and working folder) into
`PlannedStep`s: a program to run, a program to launch, or a file to copy. Variables such
as `{bsp}` are expanded here, so the plan is plain data that tests check without running
anything. Presets live in `%APPDATA%\Mallet\compile_presets.json`.

`CompileJob` runs a plan on its own thread. Each program runs without a console window
(below normal priority when the option is on), its stdout and stderr are read on two more
threads, and lines go into a shared `JobState` that the window, the status bar and MCP
read. Progress comes from the compilers' `0...1...2` counters. Lines that read as errors
or warnings become `Problem`s with the brush id or position they name. After VBSP, a new
`.lin` pointfile (or "leaked" in the output) marks a leak: the compile stops and the
editor draws the line as an ownerless scene mesh, which every view shows but nothing can
select. Cancelling kills the running program and skips the rest.

`Config` is one serde struct with defaults for every field, so older `config.json` files
load. Tool paths left empty are found next to the game (`bin/win64` first) or, for
SourceCubemap, on `PATH` and in the usual install folders.

## Drawing

Each of the four viewport slots shows a `ViewMode`: 3D Textured, 3D Wireframe, or the 2D
Top, Front or Side view, switched by clicking the slot's label. 3D slots share the one
camera; 2D slots showing the same view share its pan and zoom. MCP tools name viewports
`camera`, `top`, `front` and `side` and act on the first slot showing that view.

All four viewports are drawn by `Renderer3D` with wgpu, using the device eframe already
created, into their own offscreen images. The window shows those images through egui, and
MCP screenshots read the same images back, so they match and work while the window is
minimized. egui only draws the light overlays on top: labels, handle squares, the marquee
and the net graph. The CPU drawing code behind the `Canvas` trait (egui painter or
tiny-skia image) remains as the fallback when no GPU renderer exists yet.

Geometry is cached on the GPU and rebuilt only when the scene's `geometry_generation`
changes, never for selection changes or camera moves:

- The camera's faces are fanned into triangles with Source's texture coordinates
  (`point · axis / scale + offset`, divided by the texture size), shaded by direction like
  Hammer's textured mode, and grouped into 2048-unit chunks. Each frame, chunks whose
  bounds are outside the view frustum are skipped. Faces are one-sided, wound
  counter-clockwise from outside with back faces culled, so brushes look solid from
  outside and open from inside, as in Hammer.
- Materials carry their alpha mode from the VMT: `$alphatest` faces are cut out in the
  shader at `$alphatestreference`, and `$translucent` faces (or `$alpha` below 1) are kept
  out of the chunks, sorted back to front each frame and blended without writing depth.
- Props are instanced: each model is uploaded once, and every frame the props whose
  bounds pass frustum culling are written into one instance buffer of model matrices,
  grouped by model. Opaque meshes draw after the brushes, blended ones after the sorted
  translucent faces. Selected props are tinted through the instance colour, and a move or
  resize drag multiplies its transform into their matrices. In the scene a prop is an
  entity whose box is its model's rotated bounds, so picking, selection and the 2D views
  treat it like any other entity.
- The 2D views share one buffer of every brush edge in world space; each view only sets an
  orthographic matrix, so panning and zooming cost almost nothing.
- Each slot remembers a fingerprint of what its image was drawn from (camera or view,
  scene generations, overlay, size, texture uploads). When nothing changed, the image is
  shown again without drawing.
- Textures stream in: new materials load and get mip-mapped on a background thread while
  faces show a grey stand-in, so opening a large map does not stall. MCP screenshots wait
  for the loads to finish.
- The camera has a draw distance (default 8192 units) that sets the projection's far
  plane, so frustum culling also drops everything beyond it. Chunks are 1024 units.
- The selection, its red outline and tool previews are small and rebuilt every frame so
  drags show live. In 3D the selection is drawn again over the cached geometry at equal
  depth, tinted red.

`get_status` reports per-frame timings and how many chunks and triangles the camera drew,
and View > Net Graph shows the same numbers in the camera view's bottom-right corner.

Brushes whose planes do not enclose a volume are skipped, as Hammer's Check Problems would
report them.

## MCP server

The editor starts an MCP server at `http://127.0.0.1:7414/mcp` (Streamable HTTP transport,
JSON responses). Mallet Options > AI agents chooses another port or turns it off;
`MALLET_MCP_PORT` overrides the port, and `0` or `off` disables it. The status bar shows the address while it runs.

Security:

- It listens on loopback only.
- Requests with a non-local `Origin` header are refused, which blocks DNS-rebinding
  attacks from web pages.
- There is no authentication, so any program on this machine can use it while Mallet runs.

Tools:

| Tool | What it does |
| --- | --- |
| `get_status` | Open map, counts, bounds, grid, camera, 2D views, undo/redo state |
| `set_game_dir` | Mount a game's content for textures and remember it |
| `new_map`, `open_map`, `save_map` | Start an empty map or load a VMF or `.mallet` map in a new tab (`focus` also brings the window to the front); save in place or to a new path, the extension picking the format |
| `export_map` | Write a copy of the map in either format without switching to it |
| `list_tabs`, `switch_tab`, `close_tab` | Work with the open map tabs; closing unsaved work needs `discard` |
| `list_entities`, `get_entity` | Browse entities, filtered by classname (`trigger_*` prefix match) |
| `set_entity_key`, `remove_entity_key` | Undoable key edits |
| `get_selection`, `select`, `clear_selection` | Read or change the selection the window shows |
| `move_selection`, `clone_selection`, `delete_selection` | Undoable selection edits; moves keep textures locked |
| `resize_selection` | Stretch the selection to a new bounding box, like dragging its handles |
| `create_block` | Box brush between two corners, like the Block tool |
| `create_entity` | Point entity of any class at a position, with extra keyvalues, like the Entity tool |
| `undo`, `redo` | Step the shared history |
| `set_grid`, `center_views` | Grid size and framing |
| `set_viewport_type` | Switch a viewport slot between 3D and 2D types |
| `set_camera`, `set_view_2d` | Aim the 3D camera, pan and zoom the 2D views |
| `screenshot` | PNG of one or all viewports, rendered offscreen, or of the whole window (`window`) |
| `set_entity_outputs` | Replace an entity's outputs |
| `describe_entity_class`, `list_entity_classes` | Entity classes from the FGD files |
| `set_fgds` | Read entity classes from chosen FGD files |
| `show_properties` | Open or close Object Properties on a tab |
| `get_settings`, `set_settings` | Read and change Mallet Options, with the programs found for each tool |
| `list_compile_presets`, `set_compile_preset`, `delete_compile_preset` | Compile presets, with each step's command for the open map |
| `compile_map` | Save and compile, returning at once or waiting up to `wait_seconds` |
| `compile_status`, `cancel_compile` | Steps, progress, problems, leak and output of the compile |
| `show_compile`, `show_options`, `show_about` | Open the compile window or Mallet Options on a tab, or Help > About |
| `list_recent_maps` | The maps opened or saved lately, latest first |
| `load_pointfile`, `clear_pointfile` | Show or hide a leak line |
| `list_materials` | The game's materials, filtered, or the ones the map uses with counts |
| `get_faces`, `select_faces`, `pick_face` | Brush faces: details, texture tool selection, and the face under a camera pixel |
| `set_faces` | Material, scale, shift, rotation, lightmap scale, alignment and justify, as one undo step |
| `replace_material` | Replace a material in the map or the selected faces |
| `show_texture_browser` | Open the texture browser with a search |
| `send_input` | Key presses, clicks and drags fed into the window as if a user made them, in a viewport or anywhere in the window (`window`, in screenshot pixels) |

`send_input` queues synthetic egui events that `MalletApp::raw_input_hook` feeds into the
window one frame at a time, so they take the same path as real input. The tool returns
once the window has handled them. Unlike the other tools it needs the window to be drawing,
so it fails if the window stays minimized. Use it to test tools and shortcuts end to end.

The repository's `.mcp.json` registers the server for MCP clients such as Claude Code.
Mallet has to be running for the tools to connect.

Adding a tool: describe it in `tools::definitions`, handle it in `tools::run_checked`, and
make any map change through `Editor::apply_edit` so it stays undoable.

## Roadmap

1. Rotate handles as `Edit`s with matching MCP tools; the Model and VisGroup tabs of
   Object Properties.
2. Model skins and bodygroups, and a model browser for prop entities.
3. Clip, vertex and displacement tools.
4. Lighting: a real-time preview first, then a bake through VRAD with lightmaps read back.
5. Entity I/O editor, instances, VisGroups and Check Problems.
