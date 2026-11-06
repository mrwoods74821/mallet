//! The tools the MCP server exposes, run against the shared [`Editor`].

use std::path::PathBuf;

use glam::{DVec2, DVec3};
use mallet_content::{ClassKind, EntityClass};
use mallet_vmf::edit::Edit;
use mallet_vmf::geometry::Bounds;
use mallet_vmf::{Connection, Entity, EntityItem};
use serde_json::{json, Value};

use eframe::egui::Pos2;

use crate::config::Config;
use crate::editor::{Editor, SelectMode, ShotTarget, Tool, ViewMode, MAX_GRID, MIN_GRID};
use crate::input::{self, InputFrame};
use crate::mcp::ToolResult;
use crate::scene::Owner;

/// How many entities `list_entities` returns when no limit is given.
const DEFAULT_LIST_LIMIT: u64 = 200;
const MAX_PITCH: f64 = 89.0;

/// The `tools/list` entries for every tool [`run`] understands.
pub fn definitions() -> Value {
    let vec3 = json!({ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 });
    let vec2 = json!({ "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 });
    let id = json!({ "type": "integer", "minimum": 0, "description": "Entity id; the world is usually 1." });
    json!([
        tool(
            "get_status",
            "Show the open map, its brush and entity counts and bounds, the grid size, the 3D camera, \
             the 2D view positions, and whether undo or redo is available.",
            json!({}),
            &[],
        ),
        tool(
            "set_game_dir",
            "Mount a Source game's content so the 3D view shows textures, and remember it for next time. \
             The folder is the one that contains gameinfo.txt, such as .../GarrysMod/garrysmod or \
             .../Half-Life 2/hl2.",
            json!({ "path": { "type": "string", "description": "Absolute path to the game folder." } }),
            &["path"],
        ),
        tool(
            "new_map",
            "Open a new tab with an empty, unsaved map and switch to it. Save it with save_map and a path.",
            json!({}),
            &[],
        ),
        tool(
            "open_map",
            "Open a .vmf file in a new tab and switch to it. An untouched empty map in the current tab is \
             replaced instead.",
            json!({ "path": { "type": "string", "description": "Absolute path to the .vmf file." } }),
            &["path"],
        ),
        tool("list_tabs", "List the open map tabs, which one is active, and which have unsaved changes.", json!({}), &[]),
        tool(
            "switch_tab",
            "Make another map tab the one every other tool works on.",
            json!({ "index": { "type": "integer", "minimum": 0 } }),
            &["index"],
        ),
        tool(
            "close_tab",
            "Close a map tab. A tab with unsaved changes is only closed with discard set to true.",
            json!({
                "index": { "type": "integer", "minimum": 0, "description": "Default: the active tab." },
                "discard": { "type": "boolean", "description": "Close even with unsaved changes." }
            }),
            &[],
        ),
        tool(
            "save_map",
            "Save the open map. Overwrites its file unless a new path is given, which then becomes the map's \
             path. A new map needs a path.",
            json!({ "path": { "type": "string", "description": "Optional absolute path to save to instead." } }),
            &[],
        ),
        tool(
            "list_entities",
            "List entities with their id, classname, targetname, origin and brush count. Includes the world.",
            json!({
                "classname": {
                    "type": "string",
                    "description": "Only list this classname. A trailing * matches a prefix, such as trigger_*."
                },
                "limit": { "type": "integer", "minimum": 1, "description": "Maximum entities to return; default 200." }
            }),
            &[],
        ),
        tool(
            "get_entity",
            "Show every key, output connection and brush id of one entity.",
            json!({ "id": id }),
            &["id"],
        ),
        tool(
            "set_entity_key",
            "Set a key on an entity, adding it if missing. Undoable.",
            json!({ "id": id, "key": { "type": "string" }, "value": { "type": "string" } }),
            &["id", "key", "value"],
        ),
        tool(
            "remove_entity_key",
            "Remove a key from an entity. Undoable.",
            json!({ "id": id, "key": { "type": "string" } }),
            &["id", "key"],
        ),
        tool(
            "get_selection",
            "List what is selected and the selection's bounds. World brushes are selected one by one; \
             entities are selected whole.",
            json!({}),
            &[],
        ),
        tool(
            "select",
            "Change the selection, the same one the editor window shows. Unknown ids are ignored.",
            json!({
                "solids": { "type": "array", "items": { "type": "integer" }, "description": "World brush ids." },
                "entities": { "type": "array", "items": { "type": "integer" }, "description": "Entity ids." },
                "mode": {
                    "type": "string",
                    "enum": ["replace", "add", "remove", "toggle"],
                    "description": "How to combine with the current selection; default replace."
                }
            }),
            &[],
        ),
        tool("clear_selection", "Deselect everything.", json!({}), &[]),
        tool(
            "move_selection",
            "Move the selection with texture lock. Entity origins move too. Undoable.",
            json!({ "offset": vec3 }),
            &["offset"],
        ),
        tool(
            "clone_selection",
            "Copy the selection with fresh ids, move the copies by offset and select them, like Shift-dragging \
             in Hammer. Undoable.",
            json!({ "offset": vec3 }),
            &["offset"],
        ),
        tool(
            "resize_selection",
            "Stretch the selection so its bounding box becomes min..max, like dragging Hammer's selection \
             handles. Brushes scale along each axis; entity origins move proportionally; textures keep their \
             world alignment. Undoable.",
            json!({ "min": vec3, "max": vec3 }),
            &["min", "max"],
        ),
        tool("delete_selection", "Delete the selected brushes and entities. Undoable.", json!({}), &[]),
        tool(
            "create_block",
            "Create an axis-aligned box brush in the world between two corners and select it. Undoable. \
             Returns the new brush id.",
            json!({
                "min": vec3,
                "max": vec3,
                "material": { "type": "string", "description": "Material for every face; default is the editor's current material." }
            }),
            &["min", "max"],
        ),
        tool(
            "create_entity",
            "Create a point entity such as info_player_start or light at a position and select it. \
             Undoable. It gets an id and \"angles\" \"0 0 0\" unless keys override them. Returns the new \
             entity id.",
            json!({
                "classname": { "type": "string" },
                "origin": vec3,
                "keys": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Extra keyvalues, such as {\"model\": \"models/props_c17/oildrum001.mdl\"}."
                }
            }),
            &["classname", "origin"],
        ),
        tool(
            "set_entity_outputs",
            "Replace all outputs (I/O connections) of an entity. Undoable. Each output fires `input` on the \
             entities named `target` when `output` happens.",
            json!({
                "id": id,
                "outputs": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "output": { "type": "string" },
                            "target": { "type": "string" },
                            "input": { "type": "string" },
                            "parameter": { "type": "string" },
                            "delay": { "type": "number" },
                            "refires": { "type": "integer", "description": "-1 (default) fires every time." }
                        },
                        "required": ["output", "target", "input"]
                    }
                }
            }),
            &["id", "outputs"],
        ),
        tool(
            "describe_entity_class",
            "Describe an entity class from the loaded FGD files: its keyvalues with types, defaults, help and \
             choices, its spawnflags, inputs and outputs, and editor size, colour and model.",
            json!({ "classname": { "type": "string" } }),
            &["classname"],
        ),
        tool(
            "list_entity_classes",
            "List entity class names from the loaded FGD files.",
            json!({
                "kind": { "type": "string", "enum": ["point", "solid"], "description": "Default point." },
                "filter": { "type": "string", "description": "Only names containing this text." }
            }),
            &[],
        ),
        tool(
            "set_fgds",
            "Read entity classes from these FGD files instead of the game's default FGD, and remember the choice.",
            json!({ "paths": { "type": "array", "items": { "type": "string" } } }),
            &["paths"],
        ),
        tool(
            "show_properties",
            "Open or close the Object Properties window for the current selection, as Alt+Enter does.",
            json!({
                "open": { "type": "boolean", "description": "Default true." },
                "tab": { "type": "string", "enum": ["keyvalues", "outputs", "inputs", "flags"] }
            }),
            &[],
        ),
        tool(
            "send_input",
            "Feed keyboard and mouse input to the editor window, frame by frame, as if a user did it, then \
             wait until it has been handled. Use it to test tools and shortcuts. The window must not be \
             minimized. Each action is one of: {\"key\": \"Enter\"} using egui key names such as A, Enter, \
             Escape, Delete, OpenBracket; {\"click\": [x, y]}; {\"drag\": [[x1, y1], [x2, y2]]}. Clicks and \
             drags need \"viewport\" (camera, top, front or side); in 2D views coordinates are world units on \
             the view's axes, in the camera view they are pixels from its top-left corner. Any action may add \
             \"modifiers\" (ctrl, shift, alt) and clicks and drags may add \"button\" (primary, secondary, middle).",
            json!({
                "actions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "key": { "type": "string" },
                            "hold_frames": { "type": "integer", "minimum": 1, "description": "Frames to hold the key before releasing it; default 1." },
                            "click": vec2,
                            "drag": { "type": "array", "items": vec2, "minItems": 2, "maxItems": 2 },
                            "viewport": { "type": "string", "enum": ["camera", "top", "front", "side"] },
                            "modifiers": { "type": "array", "items": { "type": "string", "enum": ["ctrl", "shift", "alt"] } },
                            "button": { "type": "string", "enum": ["primary", "secondary", "middle"] }
                        }
                    }
                }
            }),
            &["actions"],
        ),
        tool("undo", "Undo the last edit, whether it was made by hand or by a tool.", json!({}), &[]),
        tool("redo", "Redo the last undone edit.", json!({}), &[]),
        tool(
            "set_grid",
            "Set the 2D grid size in world units.",
            json!({ "size": { "type": "number", "description": "A power of two from 1 to 512." } }),
            &["size"],
        ),
        tool(
            "center_views",
            "Fit the whole map into the three 2D views and move the 3D camera back until the map is in view.",
            json!({}),
            &[],
        ),
        tool(
            "set_camera",
            "Move or aim the 3D camera. Source coordinates: Z is up, yaw 0 looks along +X, positive pitch looks up.",
            json!({
                "position": vec3,
                "yaw": { "type": "number", "description": "Degrees." },
                "pitch": { "type": "number", "description": "Degrees, clamped to -89..89." },
                "look_at": { "allOf": [vec3], "description": "Aim at this point after moving; overrides yaw and pitch." },
                "draw_distance": { "type": "number", "description": "How far the 3D view draws, like Hammer's back clipping plane; default 8192." }
            }),
            &[],
        ),
        tool(
            "set_view_2d",
            "Pan or zoom one 2D view. Top shows X/Y, front shows Y/Z, side shows X/Z.",
            json!({
                "view": { "type": "string", "enum": ["top", "front", "side"] },
                "center": { "allOf": [vec2], "description": "World position at the view centre, in the view's two axes." },
                "pixels_per_unit": { "type": "number", "description": "Zoom; 1 means one pixel per world unit." }
            }),
            &["view"],
        ),
        tool(
            "set_viewport_type",
            "Change what one of the four viewports shows, like clicking its label. Slots are 0 top-left,              1 top-right, 2 bottom-left, 3 bottom-right.",
            json!({
                "slot": { "type": "integer", "minimum": 0, "maximum": 3 },
                "type": { "type": "string", "enum": ["textured", "wireframe", "top", "front", "side"] }
            }),
            &["slot", "type"],
        ),
        tool(
            "screenshot",
            "Render viewports to a PNG at their on-screen size. Works while the window is covered or minimized. \
             With viewport all, the layout is camera top-left, top top-right, front bottom-left, side bottom-right; \
             viewport labels are not drawn.",
            json!({
                "viewport": {
                    "type": "string",
                    "enum": ["all", "camera", "top", "front", "side", "window"],
                    "description": "Which viewport to render; default all four. \"window\" captures the whole editor window with its menus, panels and dialogs such as Object Properties; it needs the window to be visible."
                }
            }),
            &[],
        ),
    ])
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }
    })
}

/// Runs one tool call. Failures come back as error results the agent can read.
pub fn run(editor: &mut Editor, name: &str, args: &Value) -> ToolResult {
    run_checked(editor, name, args).unwrap_or_else(ToolResult::error)
}

fn run_checked(editor: &mut Editor, name: &str, args: &Value) -> Result<ToolResult, String> {
    match name {
        "get_status" => Ok(ToolResult::json(&status_json(editor))),
        "set_game_dir" => {
            let path = PathBuf::from(string_arg(args, "path")?);
            let mounts = editor.set_game_dir(&path)?;
            let config = Config {
                game_dir: Some(path.clone()),
                fgds: Vec::new(),
            };
            config.save()?;
            Ok(ToolResult::text(format!(
                "Mounted {} with {mounts} search paths.",
                path.display()
            )))
        }
        "new_map" => {
            editor.new_tab();
            Ok(ToolResult::json(&status_json(editor)))
        }
        "open_map" => {
            editor.open_in_tab(&PathBuf::from(string_arg(args, "path")?))?;
            Ok(ToolResult::json(&status_json(editor)))
        }
        "list_tabs" => Ok(ToolResult::json(&tabs_json(editor))),
        "switch_tab" => {
            let index = args
                .get("index")
                .and_then(Value::as_u64)
                .ok_or("\"index\" must be a whole number")?;
            editor.switch_tab(index as usize)?;
            Ok(ToolResult::json(&tabs_json(editor)))
        }
        "close_tab" => {
            let index = args
                .get("index")
                .and_then(Value::as_u64)
                .map_or(editor.active_tab(), |index| index as usize);
            let discard = args.get("discard").and_then(Value::as_bool).unwrap_or(false);
            let dirty = editor
                .tabs()
                .get(index)
                .ok_or_else(|| format!("there is no tab {index}"))?
                .dirty;
            if dirty && !discard {
                return Err(format!(
                    "tab {index} has unsaved changes; save it first or pass discard: true"
                ));
            }
            editor.close_tab(index)?;
            Ok(ToolResult::json(&tabs_json(editor)))
        }
        "save_map" => {
            let path = optional_string(args, "path")?.map(PathBuf::from);
            let saved = editor.save(path)?;
            Ok(ToolResult::text(format!("Saved {}", saved.display())))
        }
        "list_entities" => Ok(ToolResult::json(&list_entities(editor, args)?)),
        "get_entity" => {
            let id = id_arg(args)?;
            let entity = editor
                .document
                .as_ref()
                .ok_or("no map is open")?
                .vmf
                .all_entities()
                .find(|entity| entity.id() == Some(id))
                .ok_or_else(|| format!("no entity has id {id}"))?;
            let mut details = entity_details(entity);
            let class = editor.entity_class(entity.classname());
            details["fgd_class"] = json!(class.map(|class| class.name.clone()));
            details["outputs"] = Value::Array(entity.connections().iter().map(connection_json).collect());
            details["inputs"] = Value::Array(
                editor
                    .inputs_to(id)
                    .iter()
                    .map(|link| {
                        let mut value = connection_json(&link.connection);
                        value["source"] = json!(link.source);
                        value["source_name"] = json!(link.source_name);
                        value
                    })
                    .collect(),
            );
            Ok(ToolResult::json(&details))
        }
        "set_entity_outputs" => {
            let id = id_arg(args)?;
            let outputs = args
                .get("outputs")
                .and_then(Value::as_array)
                .ok_or("\"outputs\" must be an array")?
                .iter()
                .enumerate()
                .map(|(index, value)| connection_arg(value).map_err(|error| format!("output {index}: {error}")))
                .collect::<Result<Vec<_>, _>>()?;
            edited(editor.set_outputs(id, outputs))
        }
        "describe_entity_class" => {
            let name = string_arg(args, "classname")?;
            let class = editor
                .entity_class(name)
                .ok_or_else(|| format!("no FGD class \"{name}\"; is a game folder with FGD files set?"))?;
            Ok(ToolResult::json(&class_json(class)))
        }
        "list_entity_classes" => {
            let data = editor.game_data.as_deref().ok_or("no FGD files are loaded")?;
            let kind = match optional_string(args, "kind")?.unwrap_or("point") {
                "point" => ClassKind::Point,
                "solid" => ClassKind::Solid,
                other => return Err(format!("unknown kind \"{other}\"")),
            };
            let filter = optional_string(args, "filter")?.unwrap_or("").to_ascii_lowercase();
            let names: Vec<&str> = data
                .class_names(kind)
                .into_iter()
                .filter(|name| name.to_ascii_lowercase().contains(&filter))
                .collect();
            Ok(ToolResult::json(&json!({ "count": names.len(), "classes": names })))
        }
        "set_fgds" => {
            let paths: Vec<PathBuf> = args
                .get("paths")
                .and_then(Value::as_array)
                .ok_or("\"paths\" must be an array of strings")?
                .iter()
                .filter_map(Value::as_str)
                .map(PathBuf::from)
                .collect();
            let count = editor.set_fgds(&paths)?;
            let mut config = Config::load();
            config.fgds = paths;
            config.save()?;
            Ok(ToolResult::text(format!("Read {count} entity classes.")))
        }
        "show_properties" => {
            editor.properties_open = args.get("open").and_then(Value::as_bool).unwrap_or(true);
            editor.properties_tab = optional_string(args, "tab")?.map(str::to_owned);
            Ok(ToolResult::text(if editor.properties_open {
                "Object Properties is open."
            } else {
                "Object Properties is closed."
            }))
        }
        "set_entity_key" => edited(editor.apply_edit(&Edit::SetKey {
            entity: id_arg(args)?,
            key: string_arg(args, "key")?.to_owned(),
            value: string_arg(args, "value")?.to_owned(),
        })),
        "remove_entity_key" => edited(editor.apply_edit(&Edit::RemoveKey {
            entity: id_arg(args)?,
            key: string_arg(args, "key")?.to_owned(),
        })),
        "get_selection" => Ok(ToolResult::json(&selection_json(editor))),
        "select" => {
            let mode = match optional_string(args, "mode")?.unwrap_or("replace") {
                "replace" => SelectMode::Replace,
                "add" => SelectMode::Add,
                "remove" => SelectMode::Remove,
                "toggle" => SelectMode::Toggle,
                other => return Err(format!("unknown mode \"{other}\"")),
            };
            let solids = id_list(args, "solids")?.into_iter().map(Owner::Solid);
            let entities = id_list(args, "entities")?.into_iter().map(Owner::Entity);
            editor.select(solids.chain(entities), mode);
            Ok(ToolResult::json(&selection_json(editor)))
        }
        "clear_selection" => {
            editor.clear_selection();
            Ok(ToolResult::text("Selection cleared."))
        }
        "move_selection" => {
            editor.move_selection(required_vec3(args, "offset")?)?;
            Ok(ToolResult::json(&selection_json(editor)))
        }
        "clone_selection" => {
            editor.clone_selection(required_vec3(args, "offset")?)?;
            Ok(ToolResult::json(&selection_json(editor)))
        }
        "resize_selection" => {
            let to = Bounds {
                min: required_vec3(args, "min")?,
                max: required_vec3(args, "max")?,
            };
            editor.resize_selection(to)?;
            Ok(ToolResult::json(&selection_json(editor)))
        }
        "delete_selection" => {
            let count = editor.delete_selection()?;
            Ok(ToolResult::text(format!(
                "Deleted {count} objects. Undo with the undo tool or Ctrl+Z."
            )))
        }
        "create_block" => {
            let min = required_vec3(args, "min")?;
            let max = required_vec3(args, "max")?;
            let previous = optional_string(args, "material")?
                .map(|material| std::mem::replace(&mut editor.material, material.to_owned()));
            let result = editor.create_block(min, max);
            if let Some(previous) = previous {
                editor.material = previous;
            }
            Ok(ToolResult::json(&json!({ "solid": result? })))
        }
        "create_entity" => {
            let classname = optional_string(args, "classname")?
                .ok_or("\"classname\" is required")?
                .to_owned();
            let origin = required_vec3(args, "origin")?;
            let mut keys = Vec::new();
            if let Some(object) = args.get("keys") {
                let object = object.as_object().ok_or("\"keys\" must be an object of strings")?;
                for (key, value) in object {
                    let value = value
                        .as_str()
                        .ok_or_else(|| format!("key \"{key}\" must have a string value"))?;
                    keys.push((key.clone(), value.to_owned()));
                }
            }
            let id = editor.create_entity(&classname, origin, &keys)?;
            Ok(ToolResult::json(&json!({ "entity": id })))
        }
        "send_input" => {
            let actions = args
                .get("actions")
                .and_then(Value::as_array)
                .ok_or("\"actions\" must be an array")?;
            let mut frames = Vec::new();
            for (index, action) in actions.iter().enumerate() {
                frames.extend(input_action(editor, action).map_err(|error| format!("action {index}: {error}"))?);
            }
            let count = frames.len();
            editor.input_queue.extend(frames);
            Ok(ToolResult::text(format!("Delivered {count} frames of input.")))
        }
        "undo" | "redo" => {
            let changed = editor.step_history(name == "redo")?;
            let text = match (changed, name) {
                (true, _) => "Done.",
                (false, "undo") => "Nothing to undo.",
                (false, _) => "Nothing to redo.",
            };
            Ok(ToolResult::text(text))
        }
        "set_grid" => {
            let size = optional_number(args, "size")?.ok_or("\"size\" is required")?;
            let valid = (MIN_GRID..=MAX_GRID).contains(&size) && size.fract() == 0.0 && (size as u32).is_power_of_two();
            if !valid {
                return Err(format!(
                    "grid size must be a power of two from {MIN_GRID} to {MAX_GRID}"
                ));
            }
            editor.grid = size;
            Ok(ToolResult::text(format!("Grid is now {size}.")))
        }
        "center_views" => {
            editor.fit_2d();
            editor.fit_3d();
            Ok(ToolResult::text("Views centred on the map."))
        }
        "set_camera" => set_camera(editor, args),
        "set_viewport_type" => {
            let slot = args
                .get("slot")
                .and_then(Value::as_u64)
                .filter(|slot| *slot < 4)
                .ok_or("\"slot\" must be 0 to 3")? as usize;
            let mode = match string_arg(args, "type")? {
                "textured" => ViewMode::Textured,
                "wireframe" => ViewMode::Wireframe,
                "top" => ViewMode::Top,
                "front" => ViewMode::Front,
                "side" => ViewMode::Side,
                other => return Err(format!("unknown viewport type \"{other}\"")),
            };
            editor.layout[slot] = mode;
            Ok(ToolResult::json(&layout_json(editor)))
        }
        "set_view_2d" => set_view_2d(editor, args),
        "screenshot" => {
            let target = match optional_string(args, "viewport")?.unwrap_or("all") {
                "all" => ShotTarget::All,
                "camera" => ShotTarget::Camera,
                "top" => ShotTarget::Top,
                "front" => ShotTarget::Front,
                "side" => ShotTarget::Side,
                "window" => return Err("window screenshots are taken by the window itself".to_owned()),
                other => return Err(format!("unknown viewport \"{other}\"")),
            };
            Ok(ToolResult::png(editor.render_png(target)?))
        }
        _ => Err(format!("unknown tool \"{name}\"")),
    }
}

fn edited(result: Result<(), String>) -> Result<ToolResult, String> {
    result.map(|()| ToolResult::text("Done. Undo with the undo tool or Ctrl+Z."))
}

fn status_json(editor: &Editor) -> Value {
    let bounds = &editor.scene.bounds;
    let views: Vec<Value> = editor
        .views
        .iter()
        .map(|view| {
            json!({
                "view": view.axis.label(),
                "center": [view.center.x, view.center.y],
                "pixels_per_unit": view.scale,
            })
        })
        .collect();
    let history = editor.document.as_ref().map(|document| &document.history);
    json!({
        "map": editor.document.as_ref().map(|document| document.display_path()),
        "solids": editor.scene.solid_count,
        "entities": editor.scene.entity_count,
        "bounds": (!bounds.is_empty()).then(|| json!({
            "min": bounds.min.to_array(),
            "max": bounds.max.to_array(),
        })),
        "grid": editor.grid,
        "game_dir": editor.game_dir().map(|dir| dir.display().to_string()),
        "textured_3d": editor.gpu.is_some(),
        "gpu": editor.gpu_info,
        "viewports": layout_json(editor),
        "tab": editor.active_tab(),
        "tab_count": editor.tabs().len(),
        "timings_ms": {
            "update": editor.timings.update_ms,
            "camera": editor.timings.camera_ms,
            "views_2d": editor.timings.views_2d_ms,
            "previous_frame_cpu": editor.timings.previous_frame_cpu_ms,
            "recent_frame_interval": {
                "min": editor.timings.interval_min_ms,
                "average": editor.timings.interval_average_ms,
                "max": editor.timings.interval_max_ms,
            },
            "recent_intervals": editor.timings.recent_intervals_ms.iter().map(|ms| (ms * 100.0).round() / 100.0).collect::<Vec<_>>(),
        },
        "camera_culling": editor.gpu.as_ref().map(|gpu| json!({
            "chunks": gpu.stats.chunks,
            "chunks_drawn": gpu.stats.chunks_drawn,
            "triangles_drawn": gpu.stats.triangles_drawn,
            "props": gpu.stats.models,
            "props_drawn": gpu.stats.models_drawn,
        })),
        "models": {
            "loaded": editor.models.values().filter(|model| model.is_some()).count(),
            "failed": editor.model_errors.len(),
            "failures": model_failures(editor),
        },
        "tool": match editor.tool {
            Tool::Select => "selection",
            Tool::Block => "block",
            Tool::Entity => "entity",
        },
        "entity_class": editor.entity_class,
        "fgd": editor.game_data.as_deref().map(|data| json!({
            "files": data.files.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
            "classes": data.len(),
        })),
        "properties_open": editor.properties_open,
        "entity_marker": editor.overlay.entity.map(|point| [point.x, point.y, point.z]),
        "selected": editor.selection.len(),
        "material": editor.material,
        "camera": {
            "position": editor.camera.position.to_array(),
            "yaw": editor.camera.yaw,
            "pitch": editor.camera.pitch,
            "fov": editor.camera.fov,
            "draw_distance": editor.camera.far,
        },
        "views_2d": views,
        "can_undo": history.is_some_and(|history| history.can_undo()),
        "can_redo": history.is_some_and(|history| history.can_redo()),
    })
}

/// Turns one `send_input` action into the frames that perform it.
fn input_action(editor: &Editor, action: &Value) -> Result<Vec<InputFrame>, String> {
    let names: Vec<&str> = match action.get("modifiers") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().ok_or("\"modifiers\" must be strings"))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("\"modifiers\" must be an array".to_owned()),
    };
    let modifiers = input::parse_modifiers(&names)?;
    if let Some(key) = optional_string(action, "key")? {
        let frames = action.get("hold_frames").and_then(Value::as_u64).unwrap_or(1) as usize;
        return input::key_press(key, modifiers, frames);
    }
    let button = input::parse_button(optional_string(action, "button")?.unwrap_or("primary"))?;
    let viewport = match string_arg(action, "viewport")? {
        "camera" => ShotTarget::Camera,
        "top" => ShotTarget::Top,
        "front" => ShotTarget::Front,
        "side" => ShotTarget::Side,
        other => return Err(format!("unknown viewport \"{other}\"")),
    };
    if let Some(at) = optional_numbers::<2>(action, "click")? {
        let at = editor.viewport_point(viewport, DVec2::from_array(at))?;
        return Ok(input::click(at, button, modifiers));
    }
    let path = match action.get("drag").and_then(Value::as_array) {
        Some(points) if points.len() == 2 => points,
        _ => return Err("an action needs \"key\", \"click\" or \"drag\" with two points".to_owned()),
    };
    let point = |value: &Value| -> Result<Pos2, String> {
        let pair = match value.as_array().map(Vec::as_slice) {
            Some([x, y]) => x.as_f64().zip(y.as_f64()),
            _ => None,
        };
        let (x, y) = pair.ok_or("drag points must be [x, y]")?;
        editor.viewport_point(viewport, DVec2::new(x, y))
    };
    Ok(input::drag(point(&path[0])?, point(&path[1])?, button, modifiers))
}

fn tabs_json(editor: &Editor) -> Value {
    let tabs: Vec<Value> = editor
        .tabs()
        .into_iter()
        .map(|tab| json!({ "title": tab.title, "path": tab.path, "unsaved": tab.dirty }))
        .collect();
    json!({ "active": editor.active_tab(), "tabs": tabs })
}

fn layout_json(editor: &Editor) -> Value {
    json!(editor.layout.iter().map(|mode| mode.label()).collect::<Vec<_>>())
}

fn selection_json(editor: &Editor) -> Value {
    let mut solids = Vec::new();
    let mut entities = Vec::new();
    for owner in &editor.selection {
        match *owner {
            Owner::Solid(id) => solids.push(id),
            Owner::Entity(id) => entities.push(id),
        }
    }
    let bounds = editor.scene.selection_bounds();
    json!({
        "solids": solids,
        "entities": entities,
        "bounds": (!bounds.is_empty()).then(|| json!({
            "min": bounds.min.to_array(),
            "max": bounds.max.to_array(),
        })),
    })
}

fn list_entities(editor: &Editor, args: &Value) -> Result<Value, String> {
    let pattern = optional_string(args, "classname")?;
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(DEFAULT_LIST_LIMIT);
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let matching: Vec<&Entity> = editor
        .document
        .as_ref()
        .ok_or("no map is open")?
        .vmf
        .all_entities()
        .filter(|entity| pattern.is_none_or(|pattern| classname_matches(pattern, entity.classname())))
        .collect();
    let entities: Vec<Value> = matching
        .iter()
        .take(limit)
        .map(|entity| entity_summary(entity))
        .collect();
    Ok(json!({
        "total": matching.len(),
        "returned": entities.len(),
        "entities": entities,
    }))
}

fn set_camera(editor: &mut Editor, args: &Value) -> Result<ToolResult, String> {
    let camera = &mut editor.camera;
    if let Some(position) = optional_numbers::<3>(args, "position")? {
        camera.position = DVec3::from_array(position);
    }
    if let Some(yaw) = optional_number(args, "yaw")? {
        camera.yaw = yaw.rem_euclid(360.0);
    }
    if let Some(pitch) = optional_number(args, "pitch")? {
        camera.pitch = pitch.clamp(-MAX_PITCH, MAX_PITCH);
    }
    if let Some(distance) = optional_number(args, "draw_distance")? {
        camera.far = distance.clamp(256.0, 1_048_576.0);
    }
    if let Some(target) = optional_numbers::<3>(args, "look_at")? {
        let direction = DVec3::from_array(target) - camera.position;
        if direction.length_squared() > 0.0 {
            camera.yaw = direction.y.atan2(direction.x).to_degrees().rem_euclid(360.0);
            camera.pitch = direction
                .z
                .atan2(direction.truncate().length())
                .to_degrees()
                .clamp(-MAX_PITCH, MAX_PITCH);
        }
    }
    Ok(ToolResult::json(&json!({
        "position": camera.position.to_array(),
        "yaw": camera.yaw,
        "pitch": camera.pitch,
        "draw_distance": camera.far,
    })))
}

fn set_view_2d(editor: &mut Editor, args: &Value) -> Result<ToolResult, String> {
    let index = match string_arg(args, "view")? {
        "top" => 0,
        "front" => 1,
        "side" => 2,
        other => return Err(format!("unknown view \"{other}\"; use top, front or side")),
    };
    let view = &mut editor.views[index];
    if let Some(center) = optional_numbers::<2>(args, "center")? {
        view.center = DVec2::from_array(center);
    }
    if let Some(scale) = optional_number(args, "pixels_per_unit")? {
        view.set_scale(scale);
    }
    Ok(ToolResult::json(&json!({
        "view": view.axis.label(),
        "center": [view.center.x, view.center.y],
        "pixels_per_unit": view.scale,
    })))
}

fn classname_matches(pattern: &str, classname: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => classname
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix)),
        None => classname.eq_ignore_ascii_case(pattern),
    }
}

fn entity_summary(entity: &Entity) -> Value {
    json!({
        "id": entity.id(),
        "classname": entity.classname(),
        "targetname": entity.get("targetname"),
        "origin": entity.origin().map(|origin| origin.to_array()),
        "solids": entity.solids().count(),
    })
}

fn connection_json(connection: &Connection) -> Value {
    json!({
        "output": connection.output,
        "target": connection.target,
        "input": connection.input,
        "parameter": connection.parameter,
        "delay": connection.delay,
        "refires": connection.refires,
    })
}

fn connection_arg(value: &Value) -> Result<Connection, String> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    let connection = Connection {
        output: text("output"),
        target: text("target"),
        input: text("input"),
        parameter: text("parameter"),
        delay: value.get("delay").and_then(Value::as_f64).unwrap_or(0.0),
        refires: value.get("refires").and_then(Value::as_i64).unwrap_or(-1) as i32,
    };
    if connection.output.is_empty() || connection.input.is_empty() {
        return Err("\"output\" and \"input\" are required".to_owned());
    }
    Ok(connection)
}

fn class_json(class: &EntityClass) -> Value {
    let properties: Vec<Value> = class
        .properties
        .iter()
        .map(|property| {
            let mut value = json!({
                "name": property.name,
                "type": property.kind,
                "display": property.display,
                "default": property.default,
                "description": property.description,
            });
            if !property.choices.is_empty() {
                value["choices"] = property
                    .choices
                    .iter()
                    .map(
                        |choice| json!({ "value": choice.value, "name": choice.name, "default_on": choice.default_on }),
                    )
                    .collect();
            }
            if property.read_only {
                value["read_only"] = json!(true);
            }
            value
        })
        .collect();
    let ports = |ports: &[mallet_content::Port]| -> Vec<Value> {
        ports
            .iter()
            .map(|port| json!({ "name": port.name, "type": port.kind, "description": port.description }))
            .collect()
    };
    json!({
        "name": class.name,
        "kind": match class.kind {
            ClassKind::Base => "base",
            ClassKind::Point => "point",
            ClassKind::Solid => "solid",
        },
        "description": class.description,
        "bases": class.bases,
        "size": class.size,
        "color": class.color,
        "model": class.studio,
        "uses_model_key": class.studio_from_model,
        "properties": properties,
        "inputs": ports(&class.inputs),
        "outputs": ports(&class.outputs),
    })
}

fn entity_details(entity: &Entity) -> Value {
    let keys: Vec<[&str; 2]> = entity
        .items
        .iter()
        .filter_map(|item| match item {
            EntityItem::Key(pair) => Some([pair.key.as_str(), pair.value.as_str()]),
            _ => None,
        })
        .collect();
    let connections: Vec<[&str; 2]> = entity
        .items
        .iter()
        .filter_map(|item| match item {
            EntityItem::Other(block) if block.name.eq_ignore_ascii_case("connections") => Some(block),
            _ => None,
        })
        .flat_map(|block| block.pairs().map(|pair| [pair.key.as_str(), pair.value.as_str()]))
        .collect();
    let solids: Vec<Option<u32>> = entity.solids().map(|solid| solid.id()).collect();
    json!({
        "id": entity.id(),
        "classname": entity.classname(),
        "keys": keys,
        "connections": connections,
        "solids": solids,
        "editor_color": entity.editor_color(),
    })
}

fn string_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("\"{key}\" must be a string"))
}

fn optional_string<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(format!("\"{key}\" must be a string")),
    }
}

fn optional_number(args: &Value, key: &str) -> Result<Option<f64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .map(Some)
            .ok_or_else(|| format!("\"{key}\" must be a number")),
    }
}

fn optional_numbers<const N: usize>(args: &Value, key: &str) -> Result<Option<[f64; N]>, String> {
    let error = || format!("\"{key}\" must be an array of {N} numbers");
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) if items.len() == N => {
            let mut numbers = [0.0; N];
            for (number, item) in numbers.iter_mut().zip(items) {
                *number = item.as_f64().ok_or_else(error)?;
            }
            Ok(Some(numbers))
        }
        Some(_) => Err(error()),
    }
}

fn required_vec3(args: &Value, key: &str) -> Result<DVec3, String> {
    optional_numbers::<3>(args, key)?
        .map(DVec3::from_array)
        .ok_or_else(|| format!("\"{key}\" is required"))
}

fn id_list(args: &Value, key: &str) -> Result<Vec<u32>, String> {
    let error = || format!("\"{key}\" must be an array of whole numbers");
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_u64().and_then(|id| u32::try_from(id).ok()).ok_or_else(error))
            .collect(),
        Some(_) => Err(error()),
    }
}

fn id_arg(args: &Value) -> Result<u32, String> {
    args.get("id")
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .ok_or_else(|| "\"id\" must be a whole number".to_owned())
}

/// Up to ten models that failed to load, with the reason, sorted by path.
fn model_failures(editor: &Editor) -> Value {
    let mut failures: Vec<(&String, &String)> = editor.model_errors.iter().collect();
    failures.sort();
    failures
        .into_iter()
        .take(10)
        .map(|(path, error)| json!({ "model": path, "error": error }))
        .collect()
}
