use std::panic::{AssertUnwindSafe, catch_unwind};

use isle_lsp::lexer::lex;
use isle_lsp::parser::parse;
use isle_lsp::syntax::{File, Node, Span};

fn node_spans(node: &Node, spans: &mut Vec<Span>) {
    spans.push(node.span());
    if let Node::List(list) = node {
        let mut previous_end = list.span.start;
        for item in &list.items {
            assert!(item.span().start >= previous_end, "children overlap or precede the list");
            previous_end = item.span().end;
            node_spans(item, spans);
        }
    }
}

fn all_spans(file: &File) -> Vec<Span> {
    let mut spans: Vec<Span> = file.comments.clone();
    spans.extend(file.errors.iter().map(|e| e.span));
    for item in &file.items {
        node_spans(item, &mut spans);
    }
    spans
}

fn check_one(source: &str) -> Result<(), String> {
    let lexed = lex(source);
    let mut expected_start = 0;
    for token in &lexed.tokens {
        if token.start != expected_start || token.end <= token.start {
            return Err(format!("lexer gap, overlap or empty token at byte {}", token.start));
        }
        expected_start = token.end;
    }
    if expected_start != source.len() {
        return Err(format!("lexer stopped at byte {expected_start} of {}", source.len()));
    }

    let file = catch_unwind(AssertUnwindSafe(|| parse(source))).map_err(|_| "parser panicked".to_owned())?;
    for span in all_spans(&file) {
        let ok = span.start <= span.end
            && span.end <= source.len()
            && source.is_char_boundary(span.start)
            && source.is_char_boundary(span.end);
        if !ok {
            return Err(format!("span {}..{} is out of bounds or splits a character", span.start, span.end));
        }
    }
    Ok(())
}

pub fn check(source: &str) -> Result<(), String> {
    check_one(source)?;
    for end in (0..source.len()).filter(|&i| source.is_char_boundary(i)) {
        check_one(&source[..end]).map_err(|e| format!("{e} (input truncated to {end} bytes)"))?;
    }
    Ok(())
}
