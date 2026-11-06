<p align="center">
  <img src="branding/mallet.svg" width="128" alt="Mallet's icon: a white mallet on a black circle">
</p>

<h1 align="center">Mallet</h1>

<p align="center">A Hammer-style level editor for Source engine VMF maps, written in Rust.</p>

Mallet opens and saves VMF files without losing anything, has its own faster `.mallet`
map format, and shows maps in Hammer's four-viewport layout, with textured brushes and prop models in 3D. It has Hammer's
Selection, Block and Entity tools, Object Properties driven by the game's FGD, map tabs,
a CompilePal-style compile window with presets, SourceCubemap and leak lines, and
Hammer's texture tool with a texture browser that loads every mounted game's materials. A
built-in MCP server lets AI agents inspect and edit maps, compile them and take
screenshots. It is a work in progress: clipping, vertex editing and displacements are
next.

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
File > Open or by dropping a `.vmf` or `.mallet` file onto the window.

## The .mallet format

Maps can be saved as VMF, which every Source tool reads, or as `.mallet`, Mallet's own
binary format. A `.mallet` file holds the same map, so nothing is lost either way, but
stores face planes and texture axes as numbers rather than text and every repeated string
once. On a 28.5 MB VMF it is 2.8 MB, opens in about a third of the time and saves about
four times faster. Save As picks the format by the extension, File > Export a Copy writes
the map in either format without switching to the copy, and Mallet tells the formats
apart by their contents, so a renamed file still opens. The compilers only read VMF, so
compiling a `.mallet` map first writes a VMF copy to a `mallet_compile` folder beside it;
the BSP and logs end up there too. The layout is described in
[crates/mallet-format](crates/mallet-format/src/lib.rs).

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
| File > Recent Maps | The 20 maps opened or saved last, the latest first |
| Ctrl+W / Ctrl+Tab / Ctrl+Shift+Tab | Close tab (asks about unsaved changes) / next tab / previous tab |
| Ctrl+Z / Ctrl+Y | Undo / redo |
| Shift+S / Shift+B / Shift+E / Shift+A | Selection tool / Block tool / Entity tool / Texture application |
| Alt+Enter or double-click | Object Properties for the selection: keyvalues, outputs, inputs and flags from the game's FGD |
| F9 / Shift+F9 | Open the compile window / compile again with the preset used last |
| Click | In 2D, click a brush's centre × or its outline to select it; in 3D, click it (or aim the crosshair at it in mouselook). Props are hit where their model is, so clicks through gaps reach what is behind; only when nothing else is hit does a prop's box count. Entities are selected whole; Ctrl+click toggles |
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
| W A S D, arrow keys | Move and turn the 3D camera. A key pressed with Ctrl or Alt, such as the S of Ctrl+S, does not move it until released |

View > Net Graph toggles a frame timing overlay in the 3D view's bottom-right corner:
frame rate, CPU time per frame, how many chunks and props culling left to draw, how many viewports
reused their last image, and how many textures are still loading. View > 3D draw distance
sets how far the 3D view draws, like Hammer's back clipping plane. For the best
speed on large maps, run the release build (`cargo build --release -p mallet`).

## Texturing

Shift+A opens the Face Edit Sheet, laid out like Hammer++'s. In the 3D view, click a face
to select it and pick up its texture (Lift+Select, as in Hammer; the Mode menu has Lift,
Select, Apply and Align to View too), Ctrl+click to add faces and Shift+click for the whole
brush. Right-click puts the current texture on a face; Alt+right-click also puts the picked
up alignment on it. Scale, shift, luxel scale, rotation and the UV vectors apply as you drag,
type, scroll or click their arrows, and a drag is one undo step. The ½ and ×2 buttons halve
or double the scale of one axis while keeping the texture pinned at the face's corner, and
the dice buttons shift each face randomly, for stairs. Justify (Fit, L, R, T, B, C, and
Fit width and Fit height, which keep the texture's proportions, with Treat as one), World and Face alignment, Apply Nodraw, smoothing groups, Hide mask and
Replace Textures work as in Hammer.

The texture browser lists every material the game mounts, including the games in Garry's
Mod's `cfg/mount.cfg` and every addon. The sidebar narrows it to particular games, addons
or folders, each with a count; the bar below has Hammer's filter, keywords, "only used
textures", Mark, Replace, Open Source and the Opaque, Translucent, Selfillum, EnvMask and
error filters. The "per row" slider sets how many textures fit across, and they resize with
the window. Thumbnails load in the background while you scroll. Mounted games can be
turned off one by one in Edit > Mallet Options > Game.

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
field of view, camera speed, mouselook sensitivity, default grid, how far the mouse wheel
scrolls the texture browser), the key bindings and the MCP server address. Settings are
saved in `%APPDATA%\Mallet\config.json`.

Where you leave the main window and every panel (Face Edit, the texture browser, Compile,
Options and the rest), and their sizes, are remembered between runs in
`%APPDATA%\Mallet\data`. Set `MALLET_PROFILE` to keep a separate layout, as test copies do.

## AI agents

While Mallet runs, its MCP server listens on `http://127.0.0.1:7414/mcp`. The included
`.mcp.json` registers it for clients such as Claude Code. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the tool list and design notes.
