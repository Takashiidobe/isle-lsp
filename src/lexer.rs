use crate::syntax::{Span, SyntaxError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Whitespace,
    Newline,
    Comment,
    BlockComment,
    LParen,
    RParen,
    At,
    Symbol,
    Int,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

impl Token {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }

    pub fn span(&self) -> Span {
        Span { start: self.start, end: self.end }
    }

    pub fn is_trivia(&self) -> bool {
        matches!(self.kind, TokenKind::Whitespace | TokenKind::Newline | TokenKind::Comment | TokenKind::BlockComment)
    }
}

pub struct Lexed {
    pub tokens: Vec<Token>,
    pub errors: Vec<SyntaxError>,
}

fn is_space(b: u8) -> bool {
    b.is_ascii_whitespace() && b != b'\n'
}

fn is_symbol_first(b: u8) -> bool {
    !matches!(b, b'-' | b'0'..=b'9' | b'(' | b')' | b';') && !b.is_ascii_whitespace()
}

fn is_symbol_other(b: u8) -> bool {
    !matches!(b, b'(' | b')' | b';' | b'@') && !b.is_ascii_whitespace()
}

pub fn lex(source: &str) -> Lexed {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut errors = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let start = pos;
        let b = bytes[pos];
        let kind = match b {
            b'\n' => {
                pos += 1;
                TokenKind::Newline
            }
            _ if is_space(b) => {
                while pos < bytes.len() && is_space(bytes[pos]) {
                    pos += 1;
                }
                TokenKind::Whitespace
            }
            b';' => {
                while pos < bytes.len() && !matches!(bytes[pos], b'\n' | b'\r') {
                    pos += 1;
                }
                TokenKind::Comment
            }
            b'(' if bytes.get(pos + 1) == Some(&b';') => {
                let (end, terminated) = block_comment_end(bytes, pos);
                pos = end;
                if !terminated {
                    errors.push(error(start, start + 2, "unterminated block comment"));
                }
                TokenKind::BlockComment
            }
            b'(' => {
                pos += 1;
                TokenKind::LParen
            }
            b')' => {
                pos += 1;
                TokenKind::RParen
            }
            b'@' => {
                pos += 1;
                TokenKind::At
            }
            b'-' | b'0'..=b'9' => {
                pos = number_end(source, pos, &mut errors);
                TokenKind::Int
            }
            _ if is_symbol_first(b) => {
                while pos < bytes.len() && is_symbol_other(bytes[pos]) {
                    pos += 1;
                }
                TokenKind::Symbol
            }
            _ => unreachable!("every byte starts a token"),
        };
        tokens.push(Token { kind, start, end: pos });
    }
    Lexed { tokens, errors }
}

fn error(start: usize, end: usize, message: &str) -> SyntaxError {
    SyntaxError { span: Span { start, end }, message: message.to_owned() }
}

fn block_comment_end(bytes: &[u8], start: usize) -> (usize, bool) {
    let mut pos = start + 2;
    let mut depth = 1usize;
    while pos < bytes.len() {
        match (bytes[pos], bytes.get(pos + 1)) {
            (b'(', Some(b';')) => {
                depth += 1;
                pos += 2;
            }
            (b';', Some(b')')) => {
                depth -= 1;
                pos += 2;
                if depth == 0 {
                    return (pos, true);
                }
            }
            _ => pos += 1,
        }
    }
    (pos, false)
}

fn number_end(source: &str, start: usize, errors: &mut Vec<SyntaxError>) -> usize {
    let bytes = source.as_bytes();
    let mut pos = start;
    let negative = bytes[pos] == b'-';
    if negative {
        pos += 1;
    }
    let mut radix = 10;
    if bytes.get(pos) == Some(&b'0') {
        match bytes.get(pos + 1) {
            Some(b'x' | b'X') => radix = 16,
            Some(b'o' | b'O') => radix = 8,
            Some(b'b' | b'B') => radix = 2,
            _ => {}
        }
        if radix != 10 {
            pos += 2;
        }
    }
    let digits_start = pos;
    while pos < bytes.len() && matches!(bytes[pos], b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' | b'_') {
        pos += 1;
    }
    let digits = source[digits_start..pos].replace('_', "");
    if let Err(err) = u128::from_str_radix(&digits, radix) {
        errors.push(error(start, pos.max(start + 1), &err.to_string()));
    }
    pos
}
