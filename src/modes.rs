use tower_lsp_server::ls_types::{Position, Range};

use crate::document::Document;
use crate::lexer::{self, TokenKind};
use crate::parser;
use crate::syntax::{self, AtomKind, FormKind, Node, Span};

pub struct Query {
    pub line: usize,
    pub col: usize,
    pub kind: String,
    pub args: String,
}

pub struct Input<'a> {
    pub source: &'a str,
    pub queries: &'a [Query],
}

pub fn render(mode: &str, input: &Input) -> Result<String, String> {
    match mode {
        "document" => render_document(input),
        "tokens" => Ok(render_tokens(&plain_text(input.source))),
        "syntax-tree" => Ok(render_syntax_tree(&plain_text(input.source))),
        "source" => Ok(plain_text(input.source)),
        _ => Err(format!("unknown mode `{mode}`")),
    }
}

struct Positions {
    line_starts: Vec<usize>,
}

impl Positions {
    fn new(source: &str) -> Positions {
        let mut line_starts = vec![0];
        line_starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
        Positions { line_starts }
    }

    fn at(&self, byte: usize) -> String {
        let line = self.line_starts.partition_point(|&s| s <= byte) - 1;
        format!("{line}:{}", byte - self.line_starts[line])
    }

    fn range(&self, span: Span) -> String {
        format!("{}..{}", self.at(span.start), self.at(span.end))
    }
}

fn render_tokens(source: &str) -> String {
    let lexed = lexer::lex(source);
    let pos = Positions::new(source);
    let mut out = String::new();
    for token in lexed.tokens.iter().filter(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)) {
        out.push_str(&format!("{} {:?} {:?}\n", pos.at(token.start), token.kind, token.text(source)));
    }
    for error in &lexed.errors {
        out.push_str(&format!("error {} {}\n", pos.range(error.span), error.message));
    }
    out
}

fn render_syntax_tree(source: &str) -> String {
    let file = parser::parse(source);
    let pos = Positions::new(source);
    let mut out = String::new();
    for node in &file.items {
        if let Node::List(list) = node {
            let kind = syntax::form_kind(source, list).map_or("unknown", FormKind::keyword);
            let state = if list.closed { "closed" } else { "unclosed" };
            out.push_str(&format!("form {kind} @{} {state}\n", pos.range(list.span)));
            for item in &list.items {
                render_node(source, &pos, item, 1, &mut out);
            }
        } else {
            render_node(source, &pos, node, 0, &mut out);
        }
    }
    for error in &file.errors {
        out.push_str(&format!("error {} {}\n", pos.range(error.span), error.message));
    }
    out.push_str(&format!("comments {}\n", file.comments.len()));
    out
}

fn render_node(source: &str, pos: &Positions, node: &Node, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    match node {
        Node::Atom(atom) => {
            let kind = match atom.kind {
                AtomKind::Symbol => "symbol",
                AtomKind::Int => "int",
                AtomKind::At => "at",
            };
            out.push_str(&format!("{indent}{kind} {} @{}\n", atom.span.text(source), pos.at(atom.span.start)));
        }
        Node::List(list) => {
            let state = if list.closed { "" } else { " unclosed" };
            out.push_str(&format!("{indent}list @{}{state}\n", pos.range(list.span)));
            for item in &list.items {
                render_node(source, pos, item, depth + 1, out);
            }
        }
    }
}

const DIRECTIVES: [&str; 8] = ["mode:", "lsp:", "check:", "sameln:", "nextln:", "unordered:", "not:", "regex:"];

fn is_directive(comment: &str) -> bool {
    let body = comment.trim_start().strip_prefix(';').map(str::trim_start);
    body.is_some_and(|c| DIRECTIVES.iter().any(|d| c.starts_with(d)))
}

fn is_caret(line: &str) -> bool {
    line.trim_start().strip_prefix(';').is_some_and(|rest| rest.trim_start().starts_with('^'))
}

pub fn plain_text(source: &str) -> String {
    source.lines().filter(|l| !is_directive(l) && !is_caret(l)).collect::<Vec<_>>().join("\n")
}

fn render_document(input: &Input) -> Result<String, String> {
    let mut out = String::new();
    let raw = Document::new(input.source);
    let mut line_starts = vec![0];
    line_starts.extend(input.source.match_indices('\n').map(|(i, _)| i + 1));
    let header = input.source.lines().take_while(|l| is_directive(l)).count();
    for query in input.queries.iter().filter(|q| q.kind == "at") {
        let offset = line_starts[query.line] + query.col;
        let lsp = raw.position(offset);
        let back = raw.offset(lsp) - line_starts[query.line];
        out.push_str(&format!(
            "at {}:{} -> utf16 {} -> byte {back}\n",
            query.line as isize - header as isize,
            query.col,
            lsp.character
        ));
    }

    let edits: Vec<&Query> = input.queries.iter().filter(|q| q.kind == "edit").collect();
    if !edits.is_empty() {
        let mut document = Document::new(&plain_text(input.source));
        for edit in edits {
            let (range, text) = parse_edit(&edit.args)?;
            document.apply(Some(range), &text);
            out.push_str(&format!("edit {}\n", edit.args));
        }
        out.push_str(&format!("text:\n{}\n", document.text()));
    }
    Ok(out)
}

pub fn parse_edit(args: &str) -> Result<(Range, String), String> {
    let bad = || format!("bad edit `{args}`, expected `L:C-L:C text`");
    let (range, text) = args.split_once(' ').unwrap_or((args, ""));
    let (start, end) = range.split_once('-').ok_or_else(bad)?;
    let position = |s: &str| -> Result<Position, String> {
        let (line, col) = s.split_once(':').ok_or_else(bad)?;
        Ok(Position::new(line.parse().map_err(|_| bad())?, col.parse().map_err(|_| bad())?))
    };
    Ok((Range::new(position(start)?, position(end)?), text.replace("\\n", "\n")))
}
