//! A VMF file on disk together with its parsed map and undo history.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use mallet_vmf::edit::History;
use mallet_vmf::{Vmf, VmfError};

/// How the file's bytes were decoded, so saving writes them back the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextEncoding {
    Utf8,
    /// Any file that is not valid UTF-8. Each byte maps to one character, so every byte
    /// survives a load and save unchanged.
    Latin1,
}

/// Why a document could not be loaded or saved.
#[derive(Debug)]
pub enum DocumentError {
    /// Reading or writing the file failed.
    Io(io::Error),
    /// The file is not a valid VMF.
    Vmf(VmfError),
    /// A new map has no file to save to yet.
    Untitled,
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Vmf(error) => error.fmt(f),
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
    /// Reads and parses a VMF file.
    pub fn load(path: &Path) -> Result<Self, DocumentError> {
        let bytes = fs::read(path)?;
        let (text, encoding) = match String::from_utf8(bytes) {
            Ok(text) => (text, TextEncoding::Utf8),
            Err(error) => (
                error.into_bytes().iter().map(|&byte| char::from(byte)).collect(),
                TextEncoding::Latin1,
            ),
        };
        Ok(Self {
            path: Some(path.to_owned()),
            vmf: Vmf::parse(&text)?,
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

    /// Writes the map to [`Document::path`] through a temporary file, so a failed save
    /// never leaves a half-written map behind.
    pub fn save(&self) -> Result<(), DocumentError> {
        let path = self.path.as_ref().ok_or(DocumentError::Untitled)?;
        let text = self.vmf.to_text();
        let bytes = match self.encoding {
            TextEncoding::Utf8 => text.into_bytes(),
            TextEncoding::Latin1 => text
                .chars()
                .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
                .collect(),
        };
        let temporary = path.with_extension("vmf.tmp");
        fs::write(&temporary, bytes)?;
        fs::rename(&temporary, path)?;
        Ok(())
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
