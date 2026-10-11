use crate::index::{DefKind, Index};
use crate::project::Loaded;
use crate::syntax::{AtomKind, FormKind, List, Node, Span, form_kind};

pub const TYPES: [&str; 7] = ["keyword", "function", "type", "enumMember", "variable", "number", "macro"];
pub const MODIFIERS: [&str; 3] = ["declaration", "readonly", "priority"];

#[derive(Debug)]
pub struct Token {
    pub span: Span,
    pub kind: usize,
    pub modifiers: u32,
}

const KEYWORD: usize = 0;
const FUNCTION: usize = 1;
const TYPE: usize = 2;
const ENUM_MEMBER: usize = 3;
const VARIABLE: usize = 4;
const NUMBER: usize = 5;
const MACRO: usize = 6;

const DECLARATION: u32 = 1;
const READONLY: u32 = 2;
const PRIORITY: u32 = 4;

pub fn tokens(index: &Index, loaded: &Loaded, file: usize) -> Vec<Token> {
    let source = &loaded.sources[file];
    let mut found = Vec::new();
    for node in &source.file.items {
        if let Node::List(list) = node {
            visit(index, file, &source.text, list, 0, &mut found);
        }
    }
    found.sort_by_key(|token| token.span.start);
    found
}

fn classify(index: &Index, file: usize, span: Span) -> Option<(usize, u32)> {
    let describe = |kind: DefKind| match kind {
        DefKind::Type => (TYPE, 0),
        DefKind::Variant => (ENUM_MEMBER, 0),
        DefKind::Term => (FUNCTION, 0),
        DefKind::Const => (VARIABLE, READONLY),
        DefKind::Macro => (MACRO, 0),
        DefKind::Rule => (FUNCTION, 0),
        DefKind::Variable => (VARIABLE, 0),
    };
    if let Some(def) = index.defs.iter().find(|def| def.file == file && def.name_span == span) {
        let (kind, modifiers) = describe(def.kind);
        return Some((kind, modifiers | DECLARATION));
    }
    let reference = index.refs.iter().find(|r| r.file == file && r.span == span)?;
    reference.def.map(|def| describe(index.defs[def].kind))
}

fn visit(index: &Index, file: usize, text: &str, list: &List, depth: usize, found: &mut Vec<Token>) {
    let rule_priority = depth == 0 && form_kind(text, list) == Some(FormKind::Rule);
    let flag_form = depth == 0 && matches!(form_kind(text, list), Some(FormKind::Decl | FormKind::Extern));
    for (position, node) in list.items.iter().enumerate() {
        match node {
            Node::List(inner) => visit(index, file, text, inner, depth + 1, found),
            Node::Atom(atom) => match atom.kind {
                AtomKind::Int => {
                    let priority = rule_priority && position <= 2;
                    found.push(Token { span: atom.span, kind: NUMBER, modifiers: if priority { PRIORITY } else { 0 } });
                }
                AtomKind::At => {}
                AtomKind::Symbol => {
                    let word = atom.span.text(text);
                    let keyword = position == 0 && (depth == 0 || matches!(word, "let" | "if-let" | "if" | "and"));
                    if keyword {
                        found.push(Token { span: atom.span, kind: KEYWORD, modifiers: 0 });
                    } else if let Some((kind, modifiers)) = classify(index, file, atom.span) {
                        found.push(Token { span: atom.span, kind, modifiers });
                    } else if flag_form
                        && matches!(word, "pure" | "multi" | "partial" | "constructor" | "extractor" | "const" | "infallible")
                    {
                        found.push(Token { span: atom.span, kind: KEYWORD, modifiers: 0 });
                    }
                }
            },
        }
    }
}
