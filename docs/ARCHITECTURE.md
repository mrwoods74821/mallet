# Mallet architecture

Mallet is a Hammer-style editor for Source engine VMF maps, written in Rust. It aims to
match Hammer's viewports, tools and default key bindings, and to let AI agents work on
maps through a built-in MCP server.

## Crates

| Crate | Purpose |
| --- | --- |
| `mallet-kv` | Reads and writes Valve KeyValues text. Keeps every key, block and duplicate in order and writes Hammer's exact layout (tabs, CRLF). |
| `mallet-vmf` | Typed VMF model on top of `mallet-kv`, brush geometry, and the edit/undo layer. No UI dependencies. |
| `mallet-content` | Game content: `gameinfo.txt` search paths, VPK archives, VMT materials and VTF textures. |
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
the camera is inside; in mouselook the ray goes through the crosshair. The camera smooths
raw mouse movement over two frames and eases its speed, using real frame times.

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
directory trees are read up front). `Materials` turns a material name into the texture
Hammer shows: `%tooltexture` first, then `$basetexture`, following `patch` materials. VTF
images are decoded at the largest mip level no bigger than 512 pixels, and loading runs on
every CPU core. Materials that cannot be found render as Hammer's pink and black checkers.

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
| `set_game_dir` | Mount a game's content for textures and remember it |
| `new_map`, `open_map`, `save_map` | Start an empty map, load a VMF, save in place or to a new path |
| `list_entities`, `get_entity` | Browse entities, filtered by classname (`trigger_*` prefix match) |
| `set_entity_key`, `remove_entity_key` | Undoable key edits |
| `get_selection`, `select`, `clear_selection` | Read or change the selection the window shows |
| `move_selection`, `clone_selection`, `delete_selection` | Undoable selection edits; moves keep textures locked |
| `resize_selection` | Stretch the selection to a new bounding box, like dragging its handles |
| `create_block` | Box brush between two corners, like the Block tool |
| `undo`, `redo` | Step the shared history |
| `set_grid`, `center_views` | Grid size and framing |
| `set_viewport_type` | Switch a viewport slot between 3D and 2D types |
| `set_camera`, `set_view_2d` | Aim the 3D camera, pan and zoom the 2D views |
| `screenshot` | PNG of one or all viewports, rendered offscreen |
| `send_input` | Key presses, clicks and drags fed into the window as if a user made them |

`send_input` queues synthetic egui events that `MalletApp::raw_input_hook` feeds into the
window one frame at a time, so they take the same path as real input. The tool returns
once the window has handled them. Unlike the other tools it needs the window to be drawing,
so it fails if the window stays minimized. Use it to test tools and shortcuts end to end.

The repository's `.mcp.json` registers the server for MCP clients such as Claude Code.
Mallet has to be running for the tools to connect.

Adding a tool: describe it in `tools::definitions`, handle it in `tools::run_checked`, and
make any map change through `Editor::apply_edit` so it stays undoable.

## Roadmap

1. Rotate handles, texture application, and entity creation,
   all as `Edit`s with matching MCP tools.
2. MDL models for prop entities, a texture browser, and streaming texture loads so the
   first frame of a large map does not wait for every texture.
3. Clip, vertex and displacement tools.
4. Lighting: a real-time preview first, then a bake through VRAD with lightmaps read back.
5. Entity I/O editor, instances, VisGroups and Check Problems.
