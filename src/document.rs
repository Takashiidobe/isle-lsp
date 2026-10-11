use ropey::Rope;
use tower_lsp_server::ls_types::{Position, Range};

use crate::parser;
use crate::syntax::File;

pub fn position_in(text: &str, offset: usize) -> Position {
    let offset = offset.min(text.len());
    let before = &text[..text.floor_char_boundary(offset)];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let col = before[line_start..].encode_utf16().count();
    Position::new(line as u32, col as u32)
}

pub struct Document {
    rope: Rope,
    text: String,
    file: File,
}

impl Document {
    pub fn new(text: &str) -> Self {
        Self { rope: Rope::from_str(text), text: text.to_owned(), file: parser::parse(text) }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn file(&self) -> &File {
        &self.file
    }

    pub fn apply(&mut self, range: Option<Range>, replacement: &str) {
        match range {
            None => self.rope = Rope::from_str(replacement),
            Some(range) => {
                let start = self.char_at(range.start);
                let end = self.char_at(range.end).max(start);
                self.rope.remove(start..end);
                self.rope.insert(start, replacement);
            }
        }
        self.text = self.rope.to_string();
        self.file = parser::parse(&self.text);
    }

    pub fn position(&self, offset: usize) -> Position {
        let char_idx = self.rope.byte_to_char(offset.min(self.rope.len_bytes()));
        let line = self.rope.char_to_line(char_idx);
        let line_start = self.rope.line_to_char(line);
        let col = self.rope.char_to_utf16_cu(char_idx) - self.rope.char_to_utf16_cu(line_start);
        Position::new(line as u32, col as u32)
    }

    pub fn offset(&self, position: Position) -> usize {
        self.rope.char_to_byte(self.char_at(position))
    }

    fn char_at(&self, position: Position) -> usize {
        let line = position.line as usize;
        if line >= self.rope.len_lines() {
            return self.rope.len_chars();
        }
        let line_start = self.rope.line_to_char(line);
        let line_end = line_start + self.line_len_chars(line);
        let wanted = self.rope.char_to_utf16_cu(line_start) + position.character as usize;
        let total = self.rope.char_to_utf16_cu(line_end);
        self.rope.utf16_cu_to_char(wanted.min(total))
    }

    fn line_len_chars(&self, line: usize) -> usize {
        let slice = self.rope.line(line);
        let mut len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
            if len > 0 && slice.char(len - 1) == '\r' {
                len -= 1;
            }
        }
        len
    }
}
