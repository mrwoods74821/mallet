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

Without a path it starts with a new, empty map, like Hammer. You can also open a map with
File > Open or by dropping a `.vmf` onto the window.

## Controls

These follow Hammer's defaults.

| Keys | Action |
| --- | --- |
| Ctrl+N / Ctrl+O / Ctrl+S | New map / open / save (a new map asks where to save) |
| Ctrl+Z / Ctrl+Y | Undo / redo |
| Shift+S / Shift+B | Selection tool / Block tool |
| Click | Select a world brush or a whole entity (Ctrl+click toggles) |
| Drag on empty space | Select everything fully inside the rectangle (Ctrl adds) |
| Drag the selection | Move it so its corner snaps to the grid, with texture lock; hold Shift on release to clone |
| Drag a white handle | Resize the selection or the pending block from that edge or corner |
| Drag with the Block tool, then Enter | Create a box brush; drag inside it to move it, drag handles in any view to resize it (its depth starts from the last block) |
| Delete / Escape / Ctrl+A | Delete selection / deselect or cancel the block / select all |
| `[` / `]` | Smaller / larger grid |
| Ctrl+E / Ctrl+Shift+E | Centre 2D views / 3D view on the map |
| Mouse wheel | Zoom a 2D view around the cursor, or move the camera forward |
| Middle-drag or Space+drag | Pan a 2D view |
| Z | Toggle mouselook in the 3D view (Escape leaves it) |
| W A S D, arrow keys | Move and turn the 3D camera |

## AI agents

While Mallet runs, its MCP server listens on `http://127.0.0.1:7414/mcp`. The included
`.mcp.json` registers it for clients such as Claude Code. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the tool list and design notes.
