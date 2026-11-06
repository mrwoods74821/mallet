use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use mallet_kv::ParseError;

/// Why game content could not be read.
#[derive(Debug)]
pub enum ContentError {
    /// Reading a file failed.
    Io { path: PathBuf, error: io::Error },
    /// A KeyValues file such as `gameinfo.txt` is malformed.
    Syntax { path: PathBuf, error: ParseError },
    /// A file is not in the format its name suggests.
    Format { path: PathBuf, message: String },
    /// The folder has no `gameinfo.txt`.
    NoGameInfo(PathBuf),
}

impl fmt::Display for ContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Self::Syntax { path, error } => write!(f, "{}: {error}", path.display()),
            Self::Format { path, message } => write!(f, "{}: {message}", path.display()),
            Self::NoGameInfo(path) => write!(f, "{} has no gameinfo.txt", path.display()),
        }
    }
}

impl Error for ContentError {}

impl ContentError {
    pub(crate) fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::Io { path: path.into(), error }
    }

    pub(crate) fn format(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::Format {
            path: path.into(),
            message: message.into(),
        }
    }
}
