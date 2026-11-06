//! Valve Pak (VPK) archives, versions 1 and 2.
//!
//! An archive is a `*_dir.vpk` file holding the directory tree, plus numbered `*_000.vpk`
//! files holding most of the data. Only the tree is read when opening; file data is read
//! on demand.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::ContentError;

const SIGNATURE: u32 = 0x55AA_1234;
/// Archive index meaning the data follows the tree inside the `_dir.vpk` file itself.
const IN_DIRECTORY_FILE: u16 = 0x7FFF;
const ENTRY_TERMINATOR: u16 = 0xFFFF;

#[derive(Debug, Clone, Copy)]
struct Entry {
    archive: u16,
    offset: u32,
    length: u32,
    preload_offset: u64,
    preload_length: u16,
}

/// An opened VPK archive.
#[derive(Debug)]
pub struct VpkArchive {
    dir_path: PathBuf,
    /// Where data stored in the `_dir.vpk` file starts.
    data_start: u64,
    entries: HashMap<String, Entry>,
}

impl VpkArchive {
    /// Opens an archive from its `_dir.vpk` file and reads its directory tree.
    pub fn open(dir_path: &Path) -> Result<Self, ContentError> {
        let io_error = |error| ContentError::io(dir_path, error);
        let mut reader = BufReader::new(File::open(dir_path).map_err(io_error)?);
        let signature = read_u32(&mut reader).map_err(io_error)?;
        if signature != SIGNATURE {
            return Err(ContentError::format(dir_path, "not a VPK directory file"));
        }
        let version = read_u32(&mut reader).map_err(io_error)?;
        let tree_size = read_u32(&mut reader).map_err(io_error)?;
        let header_size: u64 = match version {
            1 => 12,
            2 => {
                let mut rest = [0u8; 16];
                reader.read_exact(&mut rest).map_err(io_error)?;
                28
            }
            other => {
                return Err(ContentError::format(
                    dir_path,
                    format!("unsupported VPK version {other}"),
                ))
            }
        };

        let mut tree = vec![0u8; tree_size as usize];
        reader.read_exact(&mut tree).map_err(io_error)?;
        let entries =
            parse_tree(&tree, header_size).ok_or_else(|| ContentError::format(dir_path, "corrupt VPK tree"))?;
        Ok(Self {
            dir_path: dir_path.to_owned(),
            data_start: header_size + u64::from(tree_size),
            entries,
        })
    }

    /// Number of files in the archive.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the archive holds no files.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every file path in the archive, lower case with forward slashes.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Whether the archive holds a file. Paths use forward slashes and ignore case.
    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(&normalize(path))
    }

    /// Reads a file, or returns `None` when the archive does not hold it.
    pub fn read(&self, path: &str) -> Result<Option<Vec<u8>>, ContentError> {
        let Some(entry) = self.entries.get(&normalize(path)) else {
            return Ok(None);
        };
        let mut data = Vec::with_capacity(usize::from(entry.preload_length) + entry.length as usize);
        if entry.preload_length > 0 {
            read_range(
                &self.dir_path,
                entry.preload_offset,
                u64::from(entry.preload_length),
                &mut data,
            )?;
        }
        if entry.length > 0 {
            let (path, start) = if entry.archive == IN_DIRECTORY_FILE {
                (self.dir_path.clone(), self.data_start + u64::from(entry.offset))
            } else {
                (self.archive_path(entry.archive), u64::from(entry.offset))
            };
            read_range(&path, start, u64::from(entry.length), &mut data)?;
        }
        Ok(Some(data))
    }

    fn archive_path(&self, index: u16) -> PathBuf {
        let name = self
            .dir_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = name.strip_suffix("_dir.vpk").unwrap_or(&name);
        self.dir_path.with_file_name(format!("{stem}_{index:03}.vpk"))
    }
}

/// Lower-cases a path and uses forward slashes, the form VPK trees store.
pub fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
}

fn read_range(path: &Path, start: u64, length: u64, out: &mut Vec<u8>) -> Result<(), ContentError> {
    let io_error = |error| ContentError::io(path, error);
    let mut file = File::open(path).map_err(io_error)?;
    file.seek(SeekFrom::Start(start)).map_err(io_error)?;
    file.take(length).read_to_end(out).map_err(io_error)?;
    Ok(())
}

fn read_u32(reader: &mut impl Read) -> std::io::Result<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

/// Walks the extension, folder, file-name nesting of a VPK tree. `tree_start` is the
/// tree's position in the file, needed to locate preload data.
fn parse_tree(tree: &[u8], tree_start: u64) -> Option<HashMap<String, Entry>> {
    let mut cursor = Cursor { data: tree, pos: 0 };
    let mut entries = HashMap::new();
    loop {
        let extension = cursor.string()?;
        if extension.is_empty() {
            return Some(entries);
        }
        loop {
            let folder = cursor.string()?;
            if folder.is_empty() {
                break;
            }
            loop {
                let name = cursor.string()?;
                if name.is_empty() {
                    break;
                }
                let _crc = cursor.u32()?;
                let preload_length = cursor.u16()?;
                let archive = cursor.u16()?;
                let offset = cursor.u32()?;
                let length = cursor.u32()?;
                if cursor.u16()? != ENTRY_TERMINATOR {
                    return None;
                }
                let preload_offset = tree_start + cursor.pos as u64;
                cursor.skip(usize::from(preload_length))?;
                let file = if extension == " " {
                    name.to_owned()
                } else {
                    format!("{name}.{extension}")
                };
                let path = if folder == " " {
                    file
                } else {
                    format!("{folder}/{file}")
                };
                entries.insert(
                    normalize(&path),
                    Entry {
                        archive,
                        offset,
                        length,
                        preload_offset,
                        preload_length,
                    },
                );
            }
        }
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn string(&mut self) -> Option<&'a str> {
        let length = self.data.get(self.pos..)?.iter().position(|&byte| byte == 0)?;
        let text = std::str::from_utf8(&self.data[self.pos..self.pos + length]).ok()?;
        self.pos += length + 1;
        Some(text)
    }

    fn bytes<const N: usize>(&mut self) -> Option<[u8; N]> {
        let bytes = self.data.get(self.pos..self.pos + N)?.try_into().ok()?;
        self.pos += N;
        Some(bytes)
    }

    fn u16(&mut self) -> Option<u16> {
        self.bytes().map(u16::from_le_bytes)
    }

    fn u32(&mut self) -> Option<u32> {
        self.bytes().map(u32::from_le_bytes)
    }

    fn skip(&mut self, count: usize) -> Option<()> {
        (self.pos + count <= self.data.len()).then(|| self.pos += count)
    }
}
