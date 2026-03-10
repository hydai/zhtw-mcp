// Line/column position mapping for byte offsets.
//
// Pre-computes newline positions to efficiently convert byte offsets to
// (line, col) coordinates. Column values use UTF-16 code units by default
// (matching LSP spec), with optional UTF-32 (char index) mode.

/// Column encoding mode for position reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnEncoding {
    /// UTF-16 code units (LSP default). Surrogate pairs count as 2.
    Utf16,
    /// Unicode scalar values (char count). Each char counts as 1.
    Utf32,
}

/// Pre-computed newline index for fast byte-offset to (line, col) conversion.
pub struct LineIndex<'a> {
    /// Byte offsets of each line start. line_starts[0] is always 0.
    line_starts: Vec<usize>,
    /// The source text (borrowed for column computation).
    text: &'a str,
}

impl<'a> LineIndex<'a> {
    /// Build a line index from source text.
    pub fn new(text: &'a str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts, text }
    }

    /// Convert a byte offset to (line, col), both 1-based.
    ///
    /// Column is measured in UTF-16 code units by default (LSP spec).
    /// Characters outside the BMP (e.g., emoji) count as 2 UTF-16 units.
    pub fn line_col(&self, byte_offset: usize, encoding: ColumnEncoding) -> (usize, usize) {
        // Find the line: last line_start <= byte_offset.
        let line_idx = match self.line_starts.binary_search(&byte_offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };

        let line_start = self.line_starts[line_idx];
        let slice = &self.text[line_start..byte_offset.min(self.text.len())];

        let col = match encoding {
            ColumnEncoding::Utf16 => slice.encode_utf16().count(),
            ColumnEncoding::Utf32 => slice.chars().count(),
        };

        (line_idx + 1, col + 1) // 1-based
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_ascii() {
        let idx = LineIndex::new("hello world");
        assert_eq!(idx.line_col(0, ColumnEncoding::Utf16), (1, 1));
        assert_eq!(idx.line_col(5, ColumnEncoding::Utf16), (1, 6));
        assert_eq!(idx.line_col(11, ColumnEncoding::Utf16), (1, 12));
    }

    #[test]
    fn multi_line_ascii() {
        let idx = LineIndex::new("abc\ndef\nghi");
        assert_eq!(idx.line_col(0, ColumnEncoding::Utf16), (1, 1)); // 'a'
        assert_eq!(idx.line_col(3, ColumnEncoding::Utf16), (1, 4)); // '\n'
        assert_eq!(idx.line_col(4, ColumnEncoding::Utf16), (2, 1)); // 'd'
        assert_eq!(idx.line_col(8, ColumnEncoding::Utf16), (3, 1)); // 'g'
    }

    #[test]
    fn cjk_columns_utf16() {
        // CJK chars are in BMP, each 1 UTF-16 code unit but 3 UTF-8 bytes.
        let idx = LineIndex::new("你好世界");
        assert_eq!(idx.line_col(0, ColumnEncoding::Utf16), (1, 1)); // 你
        assert_eq!(idx.line_col(3, ColumnEncoding::Utf16), (1, 2)); // 好
        assert_eq!(idx.line_col(6, ColumnEncoding::Utf16), (1, 3)); // 世
        assert_eq!(idx.line_col(9, ColumnEncoding::Utf16), (1, 4)); // 界
    }

    #[test]
    fn emoji_utf16_surrogate_pair() {
        // U+1F600 (😀) is outside BMP: 4 UTF-8 bytes, 2 UTF-16 code units.
        let idx = LineIndex::new("a😀b");
        assert_eq!(idx.line_col(0, ColumnEncoding::Utf16), (1, 1)); // 'a'
        assert_eq!(idx.line_col(1, ColumnEncoding::Utf16), (1, 2)); // 😀 start
        assert_eq!(idx.line_col(5, ColumnEncoding::Utf16), (1, 4)); // 'b' (after 2 UTF-16 units for emoji)
    }

    #[test]
    fn emoji_utf32() {
        let idx = LineIndex::new("a😀b");
        assert_eq!(idx.line_col(0, ColumnEncoding::Utf32), (1, 1));
        assert_eq!(idx.line_col(1, ColumnEncoding::Utf32), (1, 2)); // 😀
        assert_eq!(idx.line_col(5, ColumnEncoding::Utf32), (1, 3)); // 'b'
    }

    #[test]
    fn mixed_ascii_cjk_multiline() {
        let idx = LineIndex::new("Hello 你好\nWorld 世界");
        // Line 1: H(0) e(1) l(2) l(3) o(4) ' '(5) 你(6) 好(9)
        assert_eq!(idx.line_col(6, ColumnEncoding::Utf16), (1, 7)); // 你
        assert_eq!(idx.line_col(9, ColumnEncoding::Utf16), (1, 8)); // 好
                                                                    // Line 2 starts at byte 13 (\n at 12)
        assert_eq!(idx.line_col(13, ColumnEncoding::Utf16), (2, 1)); // W
    }

    #[test]
    fn offset_at_end() {
        let idx = LineIndex::new("abc");
        assert_eq!(idx.line_col(3, ColumnEncoding::Utf16), (1, 4));
    }

    #[test]
    fn empty_text() {
        let idx = LineIndex::new("");
        assert_eq!(idx.line_col(0, ColumnEncoding::Utf16), (1, 1));
    }
}
