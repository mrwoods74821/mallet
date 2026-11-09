# Mallet Linux support — design (short)

Date: 2026-11-09
Status: done, marked off 2026-11-09. Built and verified on this machine
(branch linux-support, commits 0f52194, 2ce8c60, cd1562b, b57bccb): builds,
tests 66/66 green, runs on Wayland, MCP answers on :7414, Windows paths
untouched. Compile-to-BSP proven natively through the Tools++ Linux
compilers (vbsp++/vvis++/vrad++ on PATH, auto-discovered): a map built in
Mallet over MCP compiled to a valid VBSP v20, errors 0, no leak.

## Intent

Mallet builds and runs natively on Linux with full function, while every
Windows behaviour stays byte-for-byte intact. Change shape: additive,
`#[cfg]`-gated, PR-shaped (one logical commit series on a branch).

## Verified starting facts (2026-11-09)

- `cargo check --workspace` FAILS on Linux: eframe built with
  `default-features = false` never enables a winit backend; winit 0.30
  compile_errors "platform not supported". Windows needs no backend
  feature, which is why upstream never saw this.
- Only 4 `#[cfg(windows)]` sites: shell.rs (registry/associations),
  compile/runner.rs (hide console + priority), handoff.rs (AllowSetForeground),
  app.rs (a test). handoff.rs already carries the `#[cfg(not(windows))]` no-op pattern.
- Paths: `%APPDATA%\Mallet` for config/logs/data; `DEFAULT_STEAM_DIR =
  "C:/Program Files (x86)/Steam"`; tool discovery looks for `*.exe`;
  SourceCubemap at `%LOCALAPPDATA%/Programs/...`.
- Elijah's machine: Steam at `~/.steam/debian-installation`, GMod/HL2/CSS
  installed; NO native Linux compile tools in any of them (checked).
- Roadmap and architecture docs never mention Linux.

## Approaches

- A (chosen): cfg-gated native support. Per-OS default paths, XDG dirs,
  extension-less tool discovery, Linux eframe windowing features.
  Windows code paths untouched.
- B (rejected): Wine bridge to run Windows compile tools — works locally
  but adds machinery upstream would reject; Elijah approved native-only.
- C (rejected): port to a cross-platform shell layer (xdg crate, freedesktop
  portals) — larger diff than needed for parity today.

## Decisions (builder's calls)

1. eframe: on non-Windows targets add `x11` + `wayland` features
   (same version), keeping Windows dependency spec identical.
2. Config dirs via std env (no new deps): Linux config = `$XDG_CONFIG_HOME/Mallet`
   else `$HOME/.config/Mallet`; data/logs = `$XDG_DATA_HOME/Mallet` else
   `$HOME/.local/share/Mallet`. Windows keeps `%APPDATA%\Mallet` exactly.
3. Steam detection: per-OS default dir list (Linux: `~/.steam/steam`,
   `~/.local/share/Steam`), same `libraryfolders.vdf` walk (backslash
   unescape is a no-op on POSIX paths).
4. Tool discovery: same search folders, extensionless names on Linux
   (`vbsp`, `vvis`, `vrad`, `vbspinfo`, `bspzip`, `sourcecubemap`),
   including `<game>/bin`, `<game>/bin/linux64`; Hammer++ builds stay
   Windows-discovered. Compile runner: on Linux wrap the command in
   `nice -n 10` when low priority is set; no console to hide.
5. Files options page and shell registration UI/actions: rendered only on
   Windows; Linux hides them (config fields stay for round-trip).
6. Docs: README Running section gains Linux build deps + config locations;
   %APPDATA% mentions become per-OS.
7. Verification gates: `cargo test --workspace` green on Linux; app
   launches on Elijah's Wayland desktop; MCP server on :7414 answers
   `get_status`; Windows untouched by inspection (cfg diffs only).

## Left to the builder

- Where exactly each cfg seam lands (found during implementation).
- Exact Linux tool-search folder list per crate (config.rs).
