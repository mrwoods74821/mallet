# Mallet

A Hammer-style level editor for Source engine VMF maps, written in Rust.

Early work in progress. It already opens and saves VMF files without losing anything,
shows them in Hammer's four-viewport layout with wireframe brushes, and has Hammer's
Selection and Block tools for selecting, moving, cloning, deleting and creating brushes. A built-in MCP server
lets AI agents inspect the map, edit entities and take viewport screenshots.

## Running

```bash
cargo run --release -p mallet -- path/to/map.vmf
```

The 3D view shows textures from a Source game. On first start Mallet looks for Garry's
Mod, Half-Life 2, the Source SDK Base 2013 games or Counter-Strike: Source in your Steam
libraries; choose another with Tools > Game Folder... (the folder that contains
`gameinfo.txt`). The choice is saved in `%APPDATA%\Mallet\config.json`.

Prop entities with a `model` key show their model in the 3D view, textured with the
default skin and placed by `origin`, `angles` and `modelscale`. Models the game does not
have stay as entity boxes; `get_status` lists why they failed.

Without a path it starts with a new, empty map, like Hammer. You can also open a map with
File > Open or by dropping a `.vmf` onto the window.

## Layout

The window follows Hammer++: menus with an icon toolbar underneath, browser-style map
tabs, the tool palette on the left, the four viewports, and an object panel on the right
with the selection mode, current texture and selection details. Tools that are not built
yet are shown greyed out. Each tab is its own map with its own views, camera, selection
and undo history; a dot marks unsaved changes.

## Controls

These follow Hammer's defaults.

| Keys | Action |
| --- | --- |
| Ctrl+N / Ctrl+O / Ctrl+S | New map tab / open in a tab / save (a new map asks where to save) |
| Ctrl+W / Ctrl+Tab / Ctrl+Shift+Tab | Close tab (asks about unsaved changes) / next tab / previous tab |
| Ctrl+Z / Ctrl+Y | Undo / redo |
| Shift+S / Shift+B / Shift+E | Selection tool / Block tool / Entity tool |
| Alt+Enter or double-click | Object Properties for the selection: keyvalues, outputs, inputs and flags from the game's FGD |
| F9 / Shift+F9 | Open the compile window / compile again with the preset used last |
| Click | In 2D, click a brush's centre × or its outline to select it; in 3D, click it (or aim the crosshair at it in mouselook). Entities are selected whole; Ctrl+click toggles |
| Drag on empty space | Select everything fully inside the rectangle (Ctrl adds) |
| Drag the selection | Move it so its corner snaps to the grid, with texture lock; hold Shift on release to clone |
| Drag a white handle | Resize the selection or the pending block from that edge or corner |
| Entity tool: click in 3D | Create an entity of the class chosen under New entity on the brush face under the cursor |
| Entity tool: click in 2D views, then Enter | Place the new entity's marker (each view sets its two axes, snapped to the grid), then create it |
| Drag with the Block tool, then Enter | Create a box brush; drag inside it to move it, drag handles in any view to resize it (its depth starts from the last block) |
| Delete / Escape / Ctrl+A | Delete selection / deselect or cancel the block / select all |
| `[` / `]` | Smaller / larger grid |
| Ctrl+E / Ctrl+Shift+E | Centre 2D views / 3D view on the map |
| Mouse wheel | Zoom a 2D view around the cursor, or move the camera forward |
| Middle-drag or Space+drag | Pan a 2D view |
| Z | Toggle mouselook in the 3D view (Escape leaves it); the cursor stays in the view and returns afterwards |
| Click a viewport's label | Switch that viewport between 3D Textured, 3D Wireframe and the 2D Top, Front and Side views |
| W A S D, arrow keys | Move and turn the 3D camera |

View > Net Graph toggles a frame timing overlay in the 3D view's bottom-right corner:
frame rate, CPU time per frame, how many chunks and props culling left to draw, how many viewports
reused their last image, and how many textures are still loading. View > 3D draw distance
sets how far the 3D view draws, like Hammer's back clipping plane. For the best
speed on large maps, run the release build (`cargo build --release -p mallet`).

## Compiling

The hammer icon on the toolbar (or F9) opens the compile window, which works like
CompilePal. A preset is a list of steps: VBSP, VVIS, VRAD, SourceCubemap, BSPZip repack,
VBSPInfo, copying the BSP to the game, starting the game, or any program you add. Tick
each tool's options, add extra arguments, reorder steps, and keep as many presets as
you like; Fast, Normal, Final, Entities only and Cubemaps come built in. Custom steps
can use `{map}`, `{vmf}`, `{bsp}`, `{mapdir}`, `{gamedir}`, `{gamemaps}` and `{bindir}`.

While it runs, the window shows each step's progress and time and the live output.
Errors and warnings are collected on the Problems tab, with buttons that select the brush
or show the place a message names. When the map leaks, the leak line is drawn in red in
every view and the 3D camera moves to it. Logs are kept in `%APPDATA%\Mallet\logs`, and
Shift+F9 compiles again with the last preset.

[SourceCubemap](https://github.com/riggs9162/SourceCubemap) builds cubemaps without
starting the game; Mallet runs it in place on the compiled BSP so the cubemaps keep the
map's name.

## Mallet Options

Edit > Mallet Options... (or the gear on the toolbar) holds every setting: the game
folder and FGD files, the compile programs (VBSP, VVIS, VRAD, VBSPInfo, BSPZip,
SourceCubemap and the game, each found automatically, with a menu of every copy found,
including 64-bit and Hammer++ builds), how compiles run, the viewports (draw distance,
field of view, camera speed, mouselook sensitivity, default grid), the key bindings and
the MCP server address. Settings are saved in `%APPDATA%\Mallet\config.json`.

## AI agents

While Mallet runs, its MCP server listens on `http://127.0.0.1:7414/mcp`. The included
`.mcp.json` registers it for clients such as Claude Code. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the tool list and design notes.
