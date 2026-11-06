//! Compiling maps the way CompilePal does: a preset is a list of steps (VBSP, VVIS, VRAD,
//! SourceCubemap, copying to the game, launching it, or any program), each with options
//! picked from a catalog and free extra arguments. Steps can use variables such as
//! `{bsp}` and `{gamedir}`, so custom tools fit into the same chain.

pub mod runner;
mod session;

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{self, ToolKind};

pub use runner::{CompileJob, Outcome, Severity, StepStatus};
pub use session::COMPILE_FOLDER;

/// What a step does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Vbsp,
    Vvis,
    Vrad,
    Vbspinfo,
    SourceCubemap,
    /// `bspzip -repack -compress`: makes the BSP smaller.
    Repack,
    /// Copies the BSP into the game's `maps` folder.
    CopyToGame,
    /// Starts the game on the map without waiting for it.
    RunGame,
    /// Any program, with arguments that may use the variables.
    Custom,
}

impl StepKind {
    /// Every kind, in the order the "Add step" menu lists them.
    pub const ALL: [StepKind; 9] = [
        StepKind::Vbsp,
        StepKind::Vvis,
        StepKind::Vrad,
        StepKind::SourceCubemap,
        StepKind::Repack,
        StepKind::Vbspinfo,
        StepKind::CopyToGame,
        StepKind::RunGame,
        StepKind::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StepKind::Vbsp => "VBSP",
            StepKind::Vvis => "VVIS",
            StepKind::Vrad => "VRAD",
            StepKind::Vbspinfo => "VBSPInfo",
            StepKind::SourceCubemap => "SourceCubemap",
            StepKind::Repack => "Repack (BSPZip)",
            StepKind::CopyToGame => "Copy to game",
            StepKind::RunGame => "Run game",
            StepKind::Custom => "Custom program",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            StepKind::Vbsp => "Builds the BSP from the VMF: brushes, entities, props and the leak check.",
            StepKind::Vvis => "Computes visibility, so the engine only draws what can be seen.",
            StepKind::Vrad => "Bakes lightmaps and prop lighting.",
            StepKind::Vbspinfo => "Lists what the BSP contains and how close it is to the engine's limits.",
            StepKind::SourceCubemap => {
                "Renders every env_cubemap on the graphics card and builds them into the BSP, without starting the game."
            }
            StepKind::Repack => "Compresses the BSP's lumps and pakfile, making the download smaller.",
            StepKind::CopyToGame => "Copies the BSP into the game's maps folder.",
            StepKind::RunGame => "Starts the game on the map. Mallet does not wait for it.",
            StepKind::Custom => "Runs any program. Arguments and the working folder may use the variables below.",
        }
    }

    /// The program setting this step runs, if it runs a known tool.
    pub fn tool(self) -> Option<ToolKind> {
        match self {
            StepKind::Vbsp => Some(ToolKind::Vbsp),
            StepKind::Vvis => Some(ToolKind::Vvis),
            StepKind::Vrad => Some(ToolKind::Vrad),
            StepKind::Vbspinfo => Some(ToolKind::Vbspinfo),
            StepKind::SourceCubemap => Some(ToolKind::SourceCubemap),
            StepKind::Repack => Some(ToolKind::Bspzip),
            StepKind::RunGame => Some(ToolKind::Game),
            StepKind::CopyToGame | StepKind::Custom => None,
        }
    }

    /// The options the step offers as checkboxes.
    pub fn options(self) -> &'static [OptionSpec] {
        match self {
            StepKind::Vbsp => VBSP_OPTIONS,
            StepKind::Vvis => VVIS_OPTIONS,
            StepKind::Vrad => VRAD_OPTIONS,
            StepKind::SourceCubemap => SOURCECUBEMAP_OPTIONS,
            StepKind::RunGame => GAME_OPTIONS,
            _ => &[],
        }
    }
}

/// A known command-line option of a tool.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OptionSpec {
    pub flag: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// The default value when the option takes one.
    pub value: Option<&'static str>,
    /// Shown in amber when the option is risky or slow.
    pub warning: &'static str,
}

const fn option(flag: &'static str, name: &'static str, description: &'static str) -> OptionSpec {
    OptionSpec {
        flag,
        name,
        description,
        value: None,
        warning: "",
    }
}

const fn valued(flag: &'static str, name: &'static str, value: &'static str, description: &'static str) -> OptionSpec {
    OptionSpec {
        flag,
        name,
        description,
        value: Some(value),
        warning: "",
    }
}

const fn warned(spec: OptionSpec, warning: &'static str) -> OptionSpec {
    OptionSpec { warning, ..spec }
}

const VBSP_OPTIONS: &[OptionSpec] = &[
    option(
        "-onlyents",
        "Only entities",
        "Only updates the entities of an existing BSP. Fast, but brushes, props and lighting stay as they were.",
    ),
    option(
        "-onlyprops",
        "Only static props",
        "Only updates the static props of an existing BSP.",
    ),
    option(
        "-nodetail",
        "No detail brushes",
        "Leaves out func_detail brushes, to see how they affect visibility.",
    ),
    option("-nowater", "No water", "Leaves out water brushes."),
    option(
        "-noweld",
        "No welding",
        "Does not merge vertices that are very close together.",
    ),
    option("-nocsg", "No CSG", "Does not cut overlapping brushes apart."),
    option(
        "-noshare",
        "No shared edges",
        "Does not share edges, which can leave cracks between faces.",
    ),
    option(
        "-notjunc",
        "No T-junction fixes",
        "Does not fix T-junctions, which can leave sparkles between faces.",
    ),
    option(
        "-noopt",
        "No face merging",
        "Does not merge faces, so there are more of them.",
    ),
    option(
        "-nodrawtriggers",
        "Hide triggers",
        "Does not draw trigger brushes in the game.",
    ),
    option(
        "-alldetail",
        "Everything is detail",
        "Turns every brush that is not an entity into detail. For testing only.",
    ),
    option(
        "-fulldetail",
        "Detail blocks visibility",
        "Makes func_detail brushes take part in visibility.",
    ),
    valued(
        "-micro",
        "Smallest brush",
        "1.0",
        "Warns about brushes smaller than this many units.",
    ),
    option("-leaktest", "Stop on leaks", "Stops the compile when the map leaks."),
    option(
        "-staticpropcombine",
        "Combine static props",
        "Merges static props as the game's combine rules say. Needs a game that supports it.",
    ),
    option(
        "-allowdynamicpropsasstatic",
        "Dynamic props as static",
        "Allows prop_static with models made for prop_dynamic.",
    ),
    option("-low", "Low priority", "Runs VBSP below normal priority."),
    option("-verbose", "Verbose", "Prints more information."),
    valued("-threads", "Threads", "8", "How many threads to use."),
];

const VVIS_OPTIONS: &[OptionSpec] = &[
    warned(
        option(
            "-fast",
            "Fast",
            "A quick, rough visibility pass for testing. The game draws more than it needs to.",
        ),
        "Not for release: the map will run slower.",
    ),
    valued(
        "-radius_override",
        "Visibility radius",
        "2500",
        "Treats anything farther away than this many units as never visible.",
    ),
    option("-nosort", "No sorting", "Does not sort portals."),
    option("-low", "Low priority", "Runs VVIS below normal priority."),
    option("-verbose", "Verbose", "Prints more information."),
    valued("-threads", "Threads", "8", "How many threads to use."),
];

const VRAD_OPTIONS: &[OptionSpec] = &[
    option("-both", "LDR and HDR", "Bakes lighting for both LDR and HDR."),
    option("-hdr", "HDR only", "Bakes HDR lighting only."),
    option("-ldr", "LDR only", "Bakes LDR lighting only."),
    warned(
        option("-fast", "Fast", "Quick, low quality lighting for testing."),
        "Not for release: lighting will look wrong.",
    ),
    warned(
        option(
            "-final",
            "Final quality",
            "Takes more samples of the sky and area lights for smoother shadows.",
        ),
        "Much slower.",
    ),
    option(
        "-StaticPropLighting",
        "Static prop lighting",
        "Lights static props per vertex instead of from one sample.",
    ),
    option(
        "-StaticPropPolys",
        "Props cast shadows",
        "Static props block light using their real shape.",
    ),
    option(
        "-TextureShadows",
        "Texture shadows",
        "Translucent textures on props cast matching shadows. Needs the models listed in lights.rad.",
    ),
    option(
        "-StaticPropSampleScale",
        "Prop sample scale",
        "Changes how many samples props take. Some VRAD builds only.",
    ),
    valued("-bounce", "Light bounces", "100", "How many times light bounces."),
    valued(
        "-smooth",
        "Smoothing angle",
        "45",
        "Faces meeting at less than this angle get smoothed lighting.",
    ),
    valued(
        "-luxeldensity",
        "Luxel density",
        "1.0",
        "Scales the lightmap resolution of every face.",
    ),
    valued(
        "-extrasky",
        "Extra sky samples",
        "16",
        "Takes more samples of the sky. -final uses 16.",
    ),
    valued("-chop", "Chop", "4", "Smallest patch size for bounced light."),
    valued("-maxchop", "Max chop", "4", "Largest patch size for bounced light."),
    option(
        "-softsun",
        "Soft sun",
        "Softens the sun's shadows. Needs a value in some VRAD builds.",
    ),
    option(
        "-noextra",
        "No supersampling",
        "Turns off supersampling: faster, with blockier shadows.",
    ),
    option("-noskyboxrecurse", "Ignore 3D skybox", "Does not light the 3D skybox."),
    option("-dump", "Dump patches", "Writes patch data for debugging."),
    option("-low", "Low priority", "Runs VRAD below normal priority."),
    option("-verbose", "Verbose", "Prints more information."),
    valued("-threads", "Threads", "8", "How many threads to use."),
];

const SOURCECUBEMAP_OPTIONS: &[OptionSpec] = &[
    valued(
        "--quality",
        "Quality",
        "standard",
        "fast, standard or high. High renders at double resolution.",
    ),
    valued(
        "--hdr",
        "Lighting",
        "auto",
        "auto, ldr, hdr or both. Auto builds whatever lighting the map has.",
    ),
    valued(
        "--profile",
        "Game rules",
        "gmod",
        "gmod or hl2. Mallet picks one from the game folder when this is off.",
    ),
    warned(
        option(
            "--experimental",
            "Experimental",
            "Uses the closest match for anything that cannot be reproduced exactly, and lists it.",
        ),
        "Reflections are close to the game's own, not identical.",
    ),
    valued(
        "--water",
        "Water",
        "planar",
        "planar renders water reflections. Needs Experimental.",
    ),
    option(
        "--incremental",
        "Incremental",
        "Reuses cubemaps that have not changed since the last build.",
    ),
    valued(
        "--mount",
        "Extra content",
        "",
        "Adds a content folder or archive the game does not load by itself.",
    ),
    valued(
        "--device",
        "Graphics card",
        "",
        "Uses the graphics card whose name contains this text, such as 3050.",
    ),
    option(
        "--no-cache",
        "No cache",
        "Does not read or write SourceCubemap's cache.",
    ),
    option("--verbose", "Verbose", "Prints more information."),
];

const GAME_OPTIONS: &[OptionSpec] = &[
    option("-novid", "Skip intro video", "Starts without the intro video."),
    option("-console", "Console", "Opens the console on start."),
    option("-dev", "Developer mode", "Turns on developer messages."),
    option("-windowed", "Windowed", "Runs in a window."),
    option("-noborder", "No border", "Runs in a window without a border."),
    valued("-w", "Width", "1600", "Window width in pixels."),
    valued("-h", "Height", "900", "Window height in pixels."),
    option("-tools", "Tools mode", "Starts the engine's tools mode."),
    option("+sv_cheats 1", "Cheats", "Turns cheats on."),
    option("-insecure", "Insecure", "Disables Valve Anti-Cheat so local mods load."),
];

/// One option turned on in a step, with its value when it takes one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepOption {
    pub flag: String,
    #[serde(default)]
    pub value: String,
}

/// One step of a preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    pub kind: StepKind,
    pub enabled: bool,
    /// Options from the catalog that are turned on.
    pub options: Vec<StepOption>,
    /// More arguments, appended as typed. May use variables.
    pub extra_args: String,
    /// For custom steps: the name shown in the list.
    pub name: String,
    /// For custom steps: the program; for tool steps, overrides Mallet Options.
    pub program: Option<PathBuf>,
    /// For custom steps: the arguments, which may use variables.
    pub args: String,
    /// For custom steps: the working folder, which may use variables. Empty for the
    /// map's folder.
    pub working_dir: String,
    /// Keep compiling when this step fails.
    pub continue_on_error: bool,
}

impl Default for Step {
    fn default() -> Self {
        Self::new(StepKind::Custom)
    }
}

impl Step {
    pub fn new(kind: StepKind) -> Self {
        let mut step = Self {
            kind,
            enabled: true,
            options: Vec::new(),
            extra_args: String::new(),
            name: String::new(),
            program: None,
            args: String::new(),
            working_dir: String::new(),
            continue_on_error: false,
        };
        // Sensible defaults, as a fresh CompilePal step has.
        match kind {
            StepKind::SourceCubemap => step.set_option("--experimental", true),
            StepKind::RunGame => {
                for flag in ["-novid", "-console", "-dev", "-windowed"] {
                    step.set_option(flag, true);
                }
            }
            _ => {}
        }
        // A report or a game that fails to start should not mark the compile as failed.
        step.continue_on_error = matches!(kind, StepKind::Vbspinfo | StepKind::RunGame);
        step
    }

    /// The name shown in the step list.
    pub fn title(&self) -> String {
        if self.kind == StepKind::Custom && !self.name.trim().is_empty() {
            self.name.clone()
        } else {
            self.kind.label().to_owned()
        }
    }

    pub fn option(&self, flag: &str) -> Option<&StepOption> {
        self.options.iter().find(|option| option.flag == flag)
    }

    /// Turns an option on with its catalog default value, or off.
    pub fn set_option(&mut self, flag: &str, on: bool) {
        let present = self.option(flag).is_some();
        if on && !present {
            let value = self
                .kind
                .options()
                .iter()
                .find(|spec| spec.flag == flag)
                .and_then(|spec| spec.value)
                .unwrap_or("")
                .to_owned();
            self.options.push(StepOption {
                flag: flag.to_owned(),
                value,
            });
        } else if !on {
            self.options.retain(|option| option.flag != flag);
        }
    }

    pub fn with(mut self, flags: &[&str]) -> Self {
        for flag in flags {
            self.set_option(flag, true);
        }
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// A named list of steps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<Step>,
}

/// The presets a new install starts with, like CompilePal's.
pub fn default_presets() -> Vec<Preset> {
    let tail = || vec![Step::new(StepKind::CopyToGame), Step::new(StepKind::RunGame).disabled()];
    let preset = |name: &str, description: &str, mut steps: Vec<Step>| {
        steps.extend(tail());
        Preset {
            name: name.to_owned(),
            description: description.to_owned(),
            steps,
        }
    };
    vec![
        preset(
            "Fast",
            "Quick test compile: rough visibility and lighting.",
            vec![
                Step::new(StepKind::Vbsp),
                Step::new(StepKind::Vvis).with(&["-fast"]),
                Step::new(StepKind::Vrad).with(&["-fast", "-ldr"]),
            ],
        ),
        preset(
            "Normal",
            "Full visibility and lighting for LDR and HDR.",
            vec![
                Step::new(StepKind::Vbsp),
                Step::new(StepKind::Vvis),
                Step::new(StepKind::Vrad).with(&["-both"]),
                Step::new(StepKind::SourceCubemap).disabled(),
            ],
        ),
        preset(
            "Final",
            "Release quality: final lighting, prop lighting and shadows, cubemaps and a compressed BSP.",
            vec![
                Step::new(StepKind::Vbsp),
                Step::new(StepKind::Vvis),
                Step::new(StepKind::Vrad).with(&[
                    "-both",
                    "-final",
                    "-StaticPropLighting",
                    "-StaticPropPolys",
                    "-TextureShadows",
                ]),
                Step::new(StepKind::SourceCubemap).with(&["--quality"]),
                Step::new(StepKind::Repack),
                Step::new(StepKind::Vbspinfo),
            ],
        ),
        preset(
            "Entities only",
            "Updates only the entities of the last compile. Seconds instead of minutes.",
            vec![Step::new(StepKind::Vbsp).with(&["-onlyents"])],
        ),
        preset(
            "Cubemaps",
            "Rebuilds the cubemaps of the last compile with SourceCubemap.",
            vec![Step::new(StepKind::SourceCubemap)],
        ),
    ]
}

fn presets_path() -> Option<PathBuf> {
    config::data_dir().map(|dir| dir.join("compile_presets.json"))
}

/// The saved presets, or the defaults when none are saved.
pub fn load_presets() -> Vec<Preset> {
    presets_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Vec<Preset>>(&text).ok())
        .filter(|presets| !presets.is_empty())
        .unwrap_or_else(default_presets)
}

pub fn save_presets(presets: &[Preset]) -> Result<(), String> {
    let path = presets_path().ok_or("APPDATA is not set, so presets cannot be saved")?;
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(presets).map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| error.to_string())
}

/// The variables steps may use, with what they mean.
pub const VARIABLES: [(&str, &str); 7] = [
    ("{map}", "Map name without extension"),
    ("{vmf}", "Full path of the VMF"),
    ("{bsp}", "Full path of the compiled BSP"),
    ("{mapdir}", "Folder of the VMF and BSP"),
    ("{gamedir}", "Game folder holding gameinfo.txt"),
    ("{gamemaps}", "The game's maps folder"),
    ("{bindir}", "The game's bin folder"),
];

/// What a compile is about: the map and the game.
#[derive(Debug, Clone, PartialEq)]
pub struct Context {
    pub vmf: PathBuf,
    pub game_dir: PathBuf,
}

impl Context {
    pub fn map_name(&self) -> String {
        self.vmf
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn map_dir(&self) -> PathBuf {
        self.vmf.parent().map(Path::to_owned).unwrap_or_default()
    }

    pub fn bsp(&self) -> PathBuf {
        self.vmf.with_extension("bsp")
    }

    pub fn game_maps(&self) -> PathBuf {
        self.game_dir.join("maps")
    }

    /// Replaces the variables in `text`.
    pub fn expand(&self, text: &str) -> String {
        let bin = self.game_dir.parent().map(|root| root.join("bin")).unwrap_or_default();
        text.replace("{map}", &self.map_name())
            .replace("{vmf}", &display(&self.vmf))
            .replace("{bsp}", &display(&self.bsp()))
            .replace("{mapdir}", &display(&self.map_dir()))
            .replace("{gamedir}", &display(&self.game_dir))
            .replace("{gamemaps}", &display(&self.game_maps()))
            .replace("{bindir}", &display(&bin))
    }

    /// Which SourceCubemap rules fit the game.
    fn cubemap_profile(&self) -> &'static str {
        let is_gmod = self
            .game_dir
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("garrysmod"));
        if is_gmod {
            "gmod"
        } else {
            "hl2"
        }
    }
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

/// A program to run, with its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
}

impl CommandLine {
    /// The command as one line, quoted the way a shell would need it.
    pub fn display(&self) -> String {
        std::iter::once(display(&self.program))
            .chain(self.args.iter().cloned())
            .map(|part| {
                if part.contains(' ') || part.is_empty() {
                    format!("\"{part}\"")
                } else {
                    part
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// What one step of a compile will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Run a program and wait for it.
    Run(CommandLine),
    /// Start a program and move on.
    Launch(CommandLine),
    /// Copy a file.
    Copy { from: PathBuf, to: PathBuf },
}

impl Action {
    pub fn display(&self) -> String {
        match self {
            Action::Run(command) | Action::Launch(command) => command.display(),
            Action::Copy { from, to } => format!("copy \"{}\" \"{}\"", from.display(), to.display()),
        }
    }
}

/// A step ready to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedStep {
    pub title: String,
    pub kind: StepKind,
    pub action: Action,
    pub continue_on_error: bool,
}

/// Splits arguments at spaces, keeping "quoted parts" together.
pub fn split_args(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

/// The arguments of a step's turned-on options, with variables expanded. Options such as
/// `+sv_cheats 1` that contain a space become two arguments.
fn option_args(step: &Step, context: &Context) -> Vec<String> {
    let mut args = Vec::new();
    for option in &step.options {
        args.extend(option.flag.split_whitespace().map(str::to_owned));
        let takes_value = step
            .kind
            .options()
            .iter()
            .find(|spec| spec.flag == option.flag)
            .is_some_and(|spec| spec.value.is_some());
        if takes_value {
            let value = context.expand(option.value.trim());
            if !value.is_empty() {
                args.push(value);
            }
        }
    }
    args.extend(split_args(&context.expand(&step.extra_args)));
    args
}

/// Works out the command for one step. `tool` finds the program for a tool kind.
pub fn plan_step(
    step: &Step,
    context: &Context,
    tool: &dyn Fn(ToolKind) -> Option<PathBuf>,
) -> Result<PlannedStep, String> {
    let program = |kind: ToolKind| -> Result<PathBuf, String> {
        step.program
            .clone()
            .filter(|path| !path.as_os_str().is_empty())
            .or_else(|| tool(kind))
            .ok_or_else(|| {
                format!(
                    "{} was not found. Set it in Edit > Mallet Options > Compile tools.",
                    kind.label()
                )
            })
    };
    let game = display(&context.game_dir);
    let bsp = display(&context.bsp());
    let no_extension = |path: PathBuf| display(&path.with_extension(""));
    let map_dir = context.map_dir();
    let run = |kind: ToolKind, mut args: Vec<String>, tail: Vec<String>| -> Result<Action, String> {
        args.extend(option_args(step, context));
        args.extend(tail);
        Ok(Action::Run(CommandLine {
            program: program(kind)?,
            args,
            working_dir: map_dir.clone(),
        }))
    };
    let action = match step.kind {
        StepKind::Vbsp | StepKind::Vvis | StepKind::Vrad => {
            let tool = step.kind.tool().expect("compilers are tools");
            run(
                tool,
                vec!["-game".to_owned(), game],
                vec![no_extension(context.vmf.clone())],
            )?
        }
        StepKind::Vbspinfo => run(ToolKind::Vbspinfo, Vec::new(), vec![bsp])?,
        StepKind::Repack => run(
            ToolKind::Bspzip,
            vec!["-repack".to_owned(), "-compress".to_owned()],
            vec![bsp],
        )?,
        StepKind::SourceCubemap => {
            let mut args = vec![
                "build".to_owned(),
                bsp,
                "--game".to_owned(),
                game,
                // Keep the map's name: the cubemaps are stored under it.
                "--in-place".to_owned(),
                "--no-backup".to_owned(),
            ];
            if step.option("--profile").is_none() {
                args.extend(["--profile".to_owned(), context.cubemap_profile().to_owned()]);
            }
            run(ToolKind::SourceCubemap, args, Vec::new())?
        }
        StepKind::CopyToGame => Action::Copy {
            from: context.bsp(),
            to: context.game_maps().join(format!("{}.bsp", context.map_name())),
        },
        StepKind::RunGame => {
            let mut args = vec!["-game".to_owned(), game];
            args.extend(option_args(step, context));
            args.extend(["+map".to_owned(), context.map_name()]);
            let program = program(ToolKind::Game)?;
            let working_dir = program.parent().map(Path::to_owned).unwrap_or_default();
            Action::Launch(CommandLine {
                program,
                args,
                working_dir,
            })
        }
        StepKind::Custom => {
            let program = step
                .program
                .clone()
                .filter(|path| !path.as_os_str().is_empty())
                .ok_or_else(|| format!("{} has no program to run.", step.title()))?;
            let program = PathBuf::from(context.expand(&display(&program)));
            let working_dir = match context.expand(step.working_dir.trim()) {
                dir if dir.is_empty() => map_dir.clone(),
                dir => PathBuf::from(dir),
            };
            let mut args = split_args(&context.expand(&step.args));
            args.extend(split_args(&context.expand(&step.extra_args)));
            Action::Run(CommandLine {
                program,
                args,
                working_dir,
            })
        }
    };
    Ok(PlannedStep {
        title: step.title(),
        kind: step.kind,
        action,
        continue_on_error: step.continue_on_error,
    })
}

/// Plans every enabled step of a preset.
pub fn plan(
    preset: &Preset,
    context: &Context,
    tool: &dyn Fn(ToolKind) -> Option<PathBuf>,
) -> Result<Vec<PlannedStep>, String> {
    let steps: Vec<PlannedStep> = preset
        .steps
        .iter()
        .filter(|step| step.enabled)
        .map(|step| plan_step(step, context, tool))
        .collect::<Result<_, _>>()?;
    if steps.is_empty() {
        return Err(format!("the preset \"{}\" has no steps turned on", preset.name));
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Context {
        Context {
            vmf: PathBuf::from("D:/maps/my map.vmf"),
            game_dir: PathBuf::from("C:/Steam/GarrysMod/garrysmod"),
        }
    }

    fn tools(kind: ToolKind) -> Option<PathBuf> {
        Some(PathBuf::from(format!("C:/bin/{}.exe", kind.key())))
    }

    #[test]
    fn splits_quoted_arguments() {
        assert_eq!(split_args(r#"-a "b c" d  "" e"#), ["-a", "b c", "d", "", "e"]);
    }

    #[test]
    fn plans_the_compilers_with_game_and_map() {
        let step = Step::new(StepKind::Vrad).with(&["-both", "-bounce"]);
        let planned = plan_step(&step, &context(), &tools).unwrap();
        let Action::Run(command) = planned.action else {
            panic!("VRAD runs")
        };
        assert_eq!(command.program, PathBuf::from("C:/bin/vrad.exe"));
        assert_eq!(
            command.args,
            [
                "-game",
                "C:/Steam/GarrysMod/garrysmod",
                "-both",
                "-bounce",
                "100",
                "D:/maps/my map"
            ]
        );
        assert!(command.display().contains("\"D:/maps/my map\""));
    }

    #[test]
    fn sourcecubemap_keeps_the_map_name_and_picks_the_profile() {
        let planned = plan_step(&Step::new(StepKind::SourceCubemap), &context(), &tools).unwrap();
        let Action::Run(command) = planned.action else {
            panic!("SourceCubemap runs")
        };
        assert_eq!(command.args[..2], ["build", "D:/maps/my map.bsp"]);
        assert!(command.args.contains(&"--in-place".to_owned()));
        let profile = command.args.iter().position(|arg| arg == "--profile").unwrap();
        assert_eq!(command.args[profile + 1], "gmod");
        assert!(command.args.contains(&"--experimental".to_owned()));
    }

    #[test]
    fn custom_steps_expand_variables() {
        let step = Step {
            program: Some(PathBuf::from("{bindir}/tool.exe")),
            args: "--in \"{bsp}\" --name {map}".to_owned(),
            working_dir: "{gamemaps}".to_owned(),
            ..Step::new(StepKind::Custom)
        };
        let planned = plan_step(&step, &context(), &tools).unwrap();
        let Action::Run(command) = planned.action else {
            panic!("custom steps run")
        };
        assert_eq!(command.program, PathBuf::from("C:/Steam/GarrysMod/bin/tool.exe"));
        assert_eq!(command.args, ["--in", "D:/maps/my map.bsp", "--name", "my", "map"]);
        assert_eq!(command.working_dir, PathBuf::from("C:/Steam/GarrysMod/garrysmod/maps"));
    }

    #[test]
    fn copy_and_run_game() {
        let copy = plan_step(&Step::new(StepKind::CopyToGame), &context(), &tools).unwrap();
        assert_eq!(
            copy.action,
            Action::Copy {
                from: PathBuf::from("D:/maps/my map.bsp"),
                to: PathBuf::from("C:/Steam/GarrysMod/garrysmod/maps/my map.bsp"),
            }
        );
        let game = plan_step(
            &Step::new(StepKind::RunGame).with(&["+sv_cheats 1"]),
            &context(),
            &tools,
        )
        .unwrap();
        let Action::Launch(command) = game.action else {
            panic!("the game is launched")
        };
        assert!(command.args.ends_with(&[
            "+sv_cheats".to_owned(),
            "1".to_owned(),
            "+map".to_owned(),
            "my map".to_owned()
        ]));
    }

    #[test]
    fn missing_tools_are_reported() {
        let error = plan_step(&Step::new(StepKind::Vbsp), &context(), &|_| None).unwrap_err();
        assert!(error.contains("VBSP was not found"));
    }

    #[test]
    fn default_presets_plan_and_round_trip() {
        let presets = default_presets();
        for preset in &presets {
            plan(preset, &context(), &tools).unwrap();
        }
        let text = serde_json::to_string(&presets).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Preset>>(&text).unwrap(), presets);
    }
}
