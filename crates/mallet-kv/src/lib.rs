//! Reader and writer for Valve's KeyValues text format as used by Hammer's VMF files.
//!
//! The tree keeps every key, value and block in file order, including duplicate keys,
//! so a parsed document can be written back without losing information. Values are kept
//! as raw strings: VMF files do not use escape sequences, so none are interpreted.

use std::error::Error;
use std::fmt;

/// Line ending Hammer uses when it saves a VMF.
const NEWLINE: &str = "\r\n";

/// A key and its raw string value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    /// The key, exactly as written in the file.
    pub key: String,
    /// The value, exactly as written in the file.
    pub value: String,
}

impl Pair {
    /// Creates a pair from anything convertible into strings.
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// One entry inside a block or at the top level of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A `"key" "value"` line.
    Pair(Pair),
    /// A named nested block.
    Block(Block),
}

/// A named block of items, such as `solid { ... }`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Block {
    /// The block's name, such as `world`, `solid` or `side`.
    pub name: String,
    /// Pairs and child blocks in file order.
    pub items: Vec<Item>,
}

impl Block {
    /// Creates an empty block.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            items: Vec::new(),
        }
    }

    /// Returns the value of the first pair whose key matches, ignoring ASCII case.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs()
            .find(|pair| pair.key.eq_ignore_ascii_case(key))
            .map(|pair| pair.value.as_str())
    }

    /// Iterates over the block's pairs, skipping child blocks.
    pub fn pairs(&self) -> impl Iterator<Item = &Pair> {
        self.items.iter().filter_map(|item| match item {
            Item::Pair(pair) => Some(pair),
            Item::Block(_) => None,
        })
    }

    /// Iterates over the block's child blocks, skipping pairs.
    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.items.iter().filter_map(|item| match item {
            Item::Block(block) => Some(block),
            Item::Pair(_) => None,
        })
    }

    /// Iterates over child blocks whose name matches, ignoring ASCII case.
    pub fn blocks_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Block> + 'a {
        self.blocks()
            .filter(move |block| block.name.eq_ignore_ascii_case(name))
    }

    /// Appends a pair to the end of the block.
    pub fn push_pair(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.items.push(Item::Pair(Pair::new(key, value)));
    }

    /// Appends a child block to the end of the block.
    pub fn push_block(&mut self, block: Block) {
        self.items.push(Item::Block(block));
    }
}

/// A syntax error, located by 1-based line and column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line of the offending token.
    pub line: usize,
    /// 1-based column of the offending token, counted in characters.
    pub column: usize,
    /// What went wrong.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}: {}", self.line, self.column, self.message)
    }
}

impl Error for ParseError {}

enum Token<'a> {
    Text(&'a str),
    Open,
    Close,
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn error(&self, at: usize, message: impl Into<String>) -> ParseError {
        let before = &self.src[..at];
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
        ParseError {
            line,
            column: before[line_start..].chars().count() + 1,
            message: message.into(),
        }
    }

    /// Skips whitespace, `//` comments and unquoted platform conditionals such as
    /// `[$X360]` or `[!$WIN32]`, which Mallet treats as always true.
    fn skip_trivia(&mut self) {
        let bytes = self.src.as_bytes();
        loop {
            while bytes.get(self.pos).is_some_and(u8::is_ascii_whitespace) {
                self.pos += 1;
            }
            if bytes[self.pos..].starts_with(b"//") {
                while bytes.get(self.pos).is_some_and(|&byte| byte != b'\n') {
                    self.pos += 1;
                }
                continue;
            }
            if bytes.get(self.pos) == Some(&b'[') {
                let length = unquoted_length(&bytes[self.pos..]);
                if bytes[self.pos + length - 1] == b']' {
                    self.pos += length;
                    continue;
                }
            }
            return;
        }
    }

    fn next(&mut self) -> Result<Option<(Token<'a>, usize)>, ParseError> {
        self.skip_trivia();
        let bytes = self.src.as_bytes();
        let start = self.pos;
        let Some(&byte) = bytes.get(start) else {
            return Ok(None);
        };
        let token = match byte {
            b'{' => {
                self.pos += 1;
                Token::Open
            }
            b'}' => {
                self.pos += 1;
                Token::Close
            }
            b'"' => {
                let body = start + 1;
                let length = bytes[body..]
                    .iter()
                    .position(|&byte| byte == b'"')
                    .ok_or_else(|| self.error(start, "quoted string is never closed"))?;
                self.pos = body + length + 1;
                Token::Text(&self.src[body..body + length])
            }
            _ => {
                let length = unquoted_length(&bytes[start..]);
                self.pos = start + length;
                Token::Text(&self.src[start..start + length])
            }
        };
        Ok(Some((token, start)))
    }
}

/// Length of the unquoted token at the start of `bytes`.
fn unquoted_length(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .position(|&byte| byte.is_ascii_whitespace() || matches!(byte, b'{' | b'}' | b'"'))
        .unwrap_or(bytes.len())
}

/// Parses a KeyValues document into its top-level items.
///
/// A leading UTF-8 byte order mark is ignored. `//` comments are skipped. Quoted strings
/// end at the next `"` and may span lines; unquoted tokens end at whitespace, braces or quotes.
pub fn parse(text: &str) -> Result<Vec<Item>, ParseError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lexer = Lexer { src: text, pos: 0 };
    parse_items(&mut lexer, None)
}

fn parse_items(lexer: &mut Lexer<'_>, open_at: Option<usize>) -> Result<Vec<Item>, ParseError> {
    let mut items = Vec::new();
    loop {
        let Some((token, at)) = lexer.next()? else {
            return match open_at {
                Some(open) => Err(lexer.error(open, "block is never closed")),
                None => Ok(items),
            };
        };
        let key = match token {
            Token::Text(key) => key,
            Token::Close if open_at.is_some() => return Ok(items),
            Token::Close => return Err(lexer.error(at, "unexpected '}'")),
            Token::Open => return Err(lexer.error(at, "block has no name")),
        };
        match lexer.next()? {
            Some((Token::Open, open)) => items.push(Item::Block(Block {
                name: key.to_owned(),
                items: parse_items(lexer, Some(open))?,
            })),
            Some((Token::Text(value), _)) => items.push(Item::Pair(Pair::new(key, value))),
            Some((Token::Close, _)) | None => {
                return Err(lexer.error(at, format!("key \"{key}\" has no value")))
            }
        }
    }
}

/// Writes items in the layout Hammer itself produces: tab indentation, CRLF line endings,
/// block names on their own line and braces on the line after.
pub fn write(items: &[Item]) -> String {
    let mut out = String::new();
    write_items(&mut out, items, 0);
    out
}

/// Writes a single block in Hammer's layout.
pub fn write_block(block: &Block) -> String {
    let mut out = String::new();
    write_block_into(&mut out, block, 0);
    out
}

fn write_items(out: &mut String, items: &[Item], depth: usize) {
    for item in items {
        match item {
            Item::Pair(pair) => {
                indent(out, depth);
                out.push('"');
                out.push_str(&pair.key);
                out.push_str("\" \"");
                out.push_str(&pair.value);
                out.push('"');
                out.push_str(NEWLINE);
            }
            Item::Block(block) => write_block_into(out, block, depth),
        }
    }
}

fn write_block_into(out: &mut String, block: &Block, depth: usize) {
    indent(out, depth);
    if needs_quotes(&block.name) {
        out.push('"');
        out.push_str(&block.name);
        out.push('"');
    } else {
        out.push_str(&block.name);
    }
    out.push_str(NEWLINE);
    indent(out, depth);
    out.push('{');
    out.push_str(NEWLINE);
    write_items(out, &block.items, depth + 1);
    indent(out, depth);
    out.push('}');
    out.push_str(NEWLINE);
}

fn needs_quotes(name: &str) -> bool {
    name.is_empty()
        || name
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b'{' | b'}' | b'"'))
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push('\t');
    }
}
