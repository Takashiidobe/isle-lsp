use tower_lsp_server::ls_types::{Position, Range};

use crate::document::Document;

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
        "source" => Ok(plain_text(input.source)),
        _ => Err(format!("unknown mode `{mode}`")),
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
