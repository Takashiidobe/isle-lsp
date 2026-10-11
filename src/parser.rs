use crate::lexer::{self, Token, TokenKind};
use crate::syntax::{Atom, AtomKind, File, FormKind, List, Node, Span, SyntaxError};

struct Open {
    start: usize,
    items: Vec<Node>,
    last_end: usize,
}

pub fn parse(source: &str) -> File {
    let lexed = lexer::lex(source);
    let mut file = File { errors: lexed.errors, ..File::default() };
    let tokens: Vec<Token> = lexed
        .tokens
        .iter()
        .copied()
        .filter(|t| {
            if matches!(t.kind, TokenKind::Comment | TokenKind::BlockComment) {
                file.comments.push(t.span());
            }
            !t.is_trivia()
        })
        .collect();

    let mut stack: Vec<Open> = Vec::new();
    for (idx, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LParen => {
                if starts_form(source, &tokens, idx) {
                    while !stack.is_empty() {
                        close_unclosed(&mut stack, &mut file);
                    }
                }
                stack.push(Open { start: token.start, items: Vec::new(), last_end: token.end });
            }
            TokenKind::RParen => match stack.pop() {
                Some(open) => {
                    let list = List { span: Span { start: open.start, end: token.end }, items: open.items, closed: true };
                    push_node(&mut stack, &mut file, Node::List(list));
                }
                None => file.errors.push(error(token.span(), "unmatched `)`")),
            },
            kind => {
                let kind = match kind {
                    TokenKind::Symbol => AtomKind::Symbol,
                    TokenKind::Int => AtomKind::Int,
                    _ => AtomKind::At,
                };
                push_node(&mut stack, &mut file, Node::Atom(Atom { kind, span: token.span() }));
            }
        }
    }
    while !stack.is_empty() {
        close_unclosed(&mut stack, &mut file);
    }

    check_top_level(source, &mut file);
    file
}

fn error(span: Span, message: &str) -> SyntaxError {
    SyntaxError { span, message: message.to_owned() }
}

fn starts_form(source: &str, tokens: &[Token], idx: usize) -> bool {
    let at_line_start = tokens[idx].start == 0 || source.as_bytes()[tokens[idx].start - 1] == b'\n';
    at_line_start
        && tokens
            .get(idx + 1)
            .is_some_and(|t| t.kind == TokenKind::Symbol && FormKind::from_keyword(t.text(source)).is_some())
}

fn push_node(stack: &mut [Open], file: &mut File, node: Node) {
    match stack.last_mut() {
        Some(open) => {
            open.last_end = node.span().end;
            open.items.push(node);
        }
        None => file.items.push(node),
    }
}

fn close_unclosed(stack: &mut Vec<Open>, file: &mut File) {
    let open = stack.pop().expect("caller checks the stack is not empty");
    file.errors.push(error(Span { start: open.start, end: open.start + 1 }, "unclosed `(`"));
    let list = List { span: Span { start: open.start, end: open.last_end }, items: open.items, closed: false };
    push_node(stack, file, Node::List(list));
}

fn check_top_level(source: &str, file: &mut File) {
    for node in &file.items {
        match node {
            Node::Atom(atom) => file.errors.push(error(atom.span, "expected a form, found a bare token")),
            Node::List(list) => match (list.items.first(), list.head()) {
                (None, _) => file.errors.push(error(list.span, "empty form")),
                (Some(first), None) => file.errors.push(error(first.span(), "expected a form name")),
                (Some(_), Some(head)) => {
                    if FormKind::from_keyword(head.span.text(source)).is_none() {
                        let message = format!("unknown form `{}`", head.span.text(source));
                        file.errors.push(error(head.span, &message));
                    }
                }
            },
        }
    }
    file.errors.sort_by_key(|e| (e.span.start, e.span.end));
}
