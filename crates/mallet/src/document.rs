//! A map file on disk, VMF or Mallet's own `.mallet`, together with its parsed map and
//! undo history.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use mallet_format::FormatError;
use mallet_vmf::edit::History;
use mallet_vmf::{Vmf, VmfError};
use serde::{Deserialize, Serialize};

/// The two kinds of map file Mallet reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapFormat {
    /// Hammer's text format, which the compilers and other editors read.
    Vmf,
    /// Mallet's binary format: the same map, several times faster to open and save and
    /// about a tenth of the size.
    Mallet,
}

impl MapFormat {
    /// The format a path's extension asks for: `.mallet`, or VMF for anything else.
    pub fn for_path(path: &Path) -> Self {
        let mallet = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(mallet_format::EXTENSION));
        if mallet {
            Self::Mallet
        } else {
            Self::Vmf
        }
    }

    /// The file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Vmf => "vmf",
            Self::Mallet => mallet_format::EXTENSION,
        }
    }

    /// The name file dialogs show.
    pub fn name(self) -> &'static str {
        match self {
            Self::Vmf => "Valve Map File",
            Self::Mallet => "Mallet Map",
        }
    }
}

/// How the file's bytes were decoded, so saving writes them back the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TextEncoding {
    #[default]
    Utf8,
    /// Any file that is not valid UTF-8. Each byte maps to one character, so every byte
    /// survives a load and save unchanged.
    Latin1,
}

/// What a `.mallet` file keeps beside the map: the editor's own notes, as JSON.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct MalletMeta {
    /// The Mallet that wrote the file, for diagnosing problems.
    writer: String,
    /// How the map's text was encoded when it came from a VMF, so exporting it to a VMF
    /// again writes the same bytes.
    text_encoding: TextEncoding,
}

/// Why a document could not be loaded or saved.
#[derive(Debug)]
pub enum DocumentError {
    /// Reading or writing the file failed.
    Io(io::Error),
    /// The file is not a valid VMF.
    Vmf(VmfError),
    /// The file is a `.mallet` map that cannot be read.
    Mallet(FormatError),
    /// A new map has no file to save to yet.
    Untitled,
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Vmf(error) => error.fmt(f),
            Self::Mallet(error) => error.fmt(f),
            Self::Untitled => write!(f, "the map has never been saved; choose where to save it"),
        }
    }
}

impl Error for DocumentError {}

impl From<io::Error> for DocumentError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<VmfError> for DocumentError {
    fn from(error: VmfError) -> Self {
        Self::Vmf(error)
    }
}

impl From<FormatError> for DocumentError {
    fn from(error: FormatError) -> Self {
        Self::Mallet(error)
    }
}

/// An open map.
pub struct Document {
    /// Where the map is saved; `None` for a new map that has never been saved.
    pub path: Option<PathBuf>,
    /// The parsed map.
    pub vmf: Vmf,
    /// Undo and redo stacks for this map.
    pub history: History,
    /// Whether the map has changed since it was opened or last saved.
    pub dirty: bool,
    encoding: TextEncoding,
}

impl Document {
    /// Reads and parses a map file. The format is told by the file's first bytes, not
    /// its extension, so a renamed file still opens.
    pub fn load(path: &Path) -> Result<Self, DocumentError> {
        let bytes = fs::read(path)?;
        let (vmf, encoding) = if mallet_format::is_mallet(&bytes) {
            let file = mallet_format::read(&bytes)?;
            let meta: MalletMeta = file
                .meta
                .as_deref()
                .and_then(|meta| serde_json::from_str(meta).ok())
                .unwrap_or_default();
            (file.vmf, meta.text_encoding)
        } else {
            let (text, encoding) = match String::from_utf8(bytes) {
                Ok(text) => (text, TextEncoding::Utf8),
                Err(error) => (
                    error.into_bytes().iter().map(|&byte| char::from(byte)).collect(),
                    TextEncoding::Latin1,
                ),
            };
            (Vmf::parse(&text)?, encoding)
        };
        Ok(Self {
            path: Some(path.to_owned()),
            vmf,
            history: History::default(),
            dirty: false,
            encoding,
        })
    }

    /// A new, empty, unsaved map.
    pub fn untitled() -> Self {
        Self {
            path: None,
            vmf: Vmf::new_map(),
            history: History::default(),
            dirty: false,
            encoding: TextEncoding::Utf8,
        }
    }

    /// Writes the map to [`Document::path`], in the format its extension names.
    pub fn save(&self) -> Result<(), DocumentError> {
        let path = self.path.as_ref().ok_or(DocumentError::Untitled)?;
        self.export(path)
    }

    /// Writes a copy of the map to `path`, in the format its extension names, without
    /// making it the document's file. The copy goes through a temporary file, so a failed
    /// write never leaves a half-written map behind.
    pub fn export(&self, path: &Path) -> Result<(), DocumentError> {
        let bytes = self.encode(MapFormat::for_path(path));
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        fs::write(&temporary, bytes)?;
        fs::rename(&temporary, path)?;
        Ok(())
    }

    /// The map as the bytes of a file in `format`.
    fn encode(&self, format: MapFormat) -> Vec<u8> {
        match format {
            MapFormat::Mallet => {
                let meta = MalletMeta {
                    writer: format!("Mallet {}", env!("CARGO_PKG_VERSION")),
                    text_encoding: self.encoding,
                };
                let meta = serde_json::to_string(&meta).expect("the editor state is plain data");
                mallet_format::write(&self.vmf, Some(&meta))
            }
            MapFormat::Vmf => {
                let text = self.vmf.to_text();
                match self.encoding {
                    TextEncoding::Utf8 => text.into_bytes(),
                    TextEncoding::Latin1 => text
                        .chars()
                        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
                        .collect(),
                }
            }
        }
    }

    /// The format the map is saved in.
    pub fn format(&self) -> MapFormat {
        self.path.as_deref().map_or(MapFormat::Vmf, MapFormat::for_path)
    }

    /// The file name shown in the title bar, or `untitled` for a new map.
    pub fn file_name(&self) -> String {
        match &self.path {
            Some(path) => path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
            None => "untitled".to_owned(),
        }
    }

    /// The full path for messages, or `untitled` for a new map.
    pub fn display_path(&self) -> String {
        self.path
            .as_ref()
            .map_or_else(|| "untitled".to_owned(), |path| path.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_between_formats_without_losing_bytes() {
        let folder = std::env::temp_dir().join(format!("mallet-document-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        // Not valid UTF-8, so the map is read as Latin-1 and must come back byte for byte.
        let text = Vmf::new_map().to_text().replace("sky_day01_01", "sky_caf\u{e9}");
        let bytes: Vec<u8> = text.chars().map(|c| u8::try_from(u32::from(c)).unwrap()).collect();
        assert!(String::from_utf8(bytes.clone()).is_err());
        let original = folder.join("map.vmf");
        fs::write(&original, &bytes).unwrap();

        let mut document = Document::load(&original).unwrap();
        let binary = folder.join("map.mallet");
        document.path = Some(binary.clone());
        document.save().unwrap();
        assert_eq!(document.format(), MapFormat::Mallet);
        assert!(mallet_format::is_mallet(&fs::read(&binary).unwrap()));

        // A `.mallet` file opens by its contents, even under another name.
        let renamed = folder.join("renamed.vmf");
        fs::copy(&binary, &renamed).unwrap();
        let reopened = Document::load(&renamed).unwrap();
        assert_eq!(reopened.vmf, document.vmf);
        let exported = folder.join("export.vmf");
        reopened.export(&exported).unwrap();
        assert_eq!(fs::read(&exported).unwrap(), bytes);
        let _ = fs::remove_dir_all(&folder);
    }
}
