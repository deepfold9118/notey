use std::io;
use std::path::{Path, PathBuf};

use codemirror_state::Text;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Ansi,
}

impl Encoding {
    pub const ALL: [Encoding; 5] =
        [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Ansi];

    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 with BOM",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::Ansi => "ANSI",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Crlf,
    Lf,
}

impl LineEnding {
    pub const ALL: [LineEnding; 2] = [LineEnding::Crlf, LineEnding::Lf];

    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Crlf => "Windows (CRLF)",
            LineEnding::Lf => "Unix (LF)",
        }
    }
}

/// A file (or untitled buffer) open in a tab. The editor state holds the
/// text; the document holds where it is saved, how it is encoded, and the
/// text as last saved, to tell whether the editor's text is modified.
pub struct Document {
    pub id: u64,
    pub path: Option<PathBuf>,
    saved: Text,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
    pub untitled_n: usize,
    /// Syntax name for highlighting; None = plain text.
    pub language: Option<String>,
    /// Last known on-disk modification time (None for untitled docs or
    /// after the file disappeared).
    pub disk_mtime: Option<std::time::SystemTime>,
}

impl Document {
    pub fn new(id: u64, untitled_n: usize) -> Self {
        Self {
            id,
            path: None,
            saved: Text::empty(),
            encoding: Encoding::Utf8,
            line_ending: LineEnding::Crlf,
            untitled_n,
            language: None,
            disk_mtime: None,
        }
    }

    /// Read a file. Returns the document and its text, with `\n` line breaks.
    pub fn open(id: u64, path: &Path) -> io::Result<(Self, String)> {
        let bytes = std::fs::read(path)?;
        let (raw, encoding) = decode(&bytes);
        let line_ending = if raw.contains("\r\n") {
            LineEnding::Crlf
        } else if raw.contains('\n') {
            LineEnding::Lf
        } else {
            LineEnding::Crlf
        };
        let text = raw.replace("\r\n", "\n").replace('\r', "\n");
        let doc = Self {
            id,
            path: Some(path.to_path_buf()),
            saved: Text::from(text.as_str()),
            encoding,
            line_ending,
            untitled_n: 0,
            language: None,
            disk_mtime: std::fs::metadata(path).and_then(|m| m.modified()).ok(),
        };
        Ok((doc, text))
    }

    /// Recreate a file-backed tab with unsaved changes from session data.
    /// The on-disk content (if readable) becomes the saved baseline so the
    /// modified indicator and Save work as expected.
    pub fn restore_file(id: u64, path: &Path) -> Self {
        match Self::open(id, path) {
            Ok((doc, _)) => doc,
            Err(_) => {
                let mut doc = Self::new(id, 0);
                doc.path = Some(path.to_path_buf());
                doc
            }
        }
    }

    pub fn title(&self) -> String {
        match &self.path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".into()),
            None => format!("Untitled {}", self.untitled_n),
        }
    }

    /// Whether `text` differs from the text as last saved.
    pub fn modified(&self, text: &Text) -> bool {
        *text != self.saved
    }

    pub fn save(&mut self, text: &Text) -> io::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no file path"))?;
        std::fs::write(&path, self.encode(&text.to_string()))?;
        self.saved = text.clone();
        self.disk_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        Ok(())
    }

    fn encode(&self, text: &str) -> Vec<u8> {
        let text = match self.line_ending {
            LineEnding::Crlf => text.replace('\n', "\r\n"),
            LineEnding::Lf => text.to_string(),
        };
        match self.encoding {
            Encoding::Utf8 => text.into_bytes(),
            Encoding::Utf8Bom => {
                let mut out = vec![0xEF, 0xBB, 0xBF];
                out.extend_from_slice(text.as_bytes());
                out
            }
            Encoding::Utf16Le => {
                let mut out = vec![0xFF, 0xFE];
                for u in text.encode_utf16() {
                    out.extend_from_slice(&u.to_le_bytes());
                }
                out
            }
            Encoding::Utf16Be => {
                let mut out = vec![0xFE, 0xFF];
                for u in text.encode_utf16() {
                    out.extend_from_slice(&u.to_be_bytes());
                }
                out
            }
            Encoding::Ansi => {
                let (bytes, _, _) = encoding_rs::WINDOWS_1252.encode(&text);
                bytes.into_owned()
            }
        }
    }
}

fn decode(bytes: &[u8]) -> (String, Encoding) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (
            String::from_utf8_lossy(&bytes[3..]).into_owned(),
            Encoding::Utf8Bom,
        )
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        (String::from_utf16_lossy(&units), Encoding::Utf16Le)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        (String::from_utf16_lossy(&units), Encoding::Utf16Be)
    } else if let Ok(s) = std::str::from_utf8(bytes) {
        (s.to_string(), Encoding::Utf8)
    } else {
        let (cow, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
        (cow.into_owned(), Encoding::Ansi)
    }
}
