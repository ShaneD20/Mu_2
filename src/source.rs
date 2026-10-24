//! Source files, byte spans and line/column lookup.

/// Identifies one file in a [`SourceMap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub u32);

/// A half-open byte range `lo..hi` within one source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub lo: u32,
    pub hi: u32,
}

impl Span {
    pub fn new(file: FileId, lo: usize, hi: usize) -> Span {
        Span {
            file,
            lo: lo as u32,
            hi: hi as u32,
        }
    }

    /// The smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        debug_assert_eq!(self.file, other.file);
        Span {
            file: self.file,
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }

    pub fn len(self) -> usize {
        (self.hi - self.lo) as usize
    }

    pub fn is_empty(self) -> bool {
        self.hi == self.lo
    }
}

/// One loaded source file with a line index for diagnostics.
pub struct SourceFile {
    pub name: String,
    pub text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceFile {
        let text = text.into();
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        SourceFile {
            name: name.into(),
            text,
            line_starts,
        }
    }

    /// 1-based line and column of a byte offset. Columns count characters.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.line_starts[line];
        let col = self
            .text
            .get(start..offset)
            .map(|s| s.chars().count())
            .unwrap_or(offset - start);
        (line + 1, col + 1)
    }

    /// The text of a 1-based line, without its line terminator.
    pub fn line_text(&self, line: usize) -> &str {
        let start = self.line_starts[line - 1];
        let end = self
            .line_starts
            .get(line)
            .map(|&e| e - 1)
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches('\r')
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }
}

/// All source files in one compilation.
#[derive(Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> SourceMap {
        SourceMap::default()
    }

    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) -> FileId {
        let id = FileId(self.files.len() as u32);
        self.files.push(SourceFile::new(name, text));
        id
    }

    pub fn get(&self, id: FileId) -> &SourceFile {
        &self.files[id.0 as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_and_column_lookup() {
        let f = SourceFile::new("t.mu", "ab\ncd\n\nef");
        assert_eq!(f.line_col(0), (1, 1));
        assert_eq!(f.line_col(1), (1, 2));
        assert_eq!(f.line_col(3), (2, 1));
        assert_eq!(f.line_col(6), (3, 1));
        assert_eq!(f.line_col(8), (4, 2));
        assert_eq!(f.line_text(2), "cd");
        assert_eq!(f.line_text(3), "");
        assert_eq!(f.line_text(4), "ef");
    }
}
