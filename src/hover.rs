use crate::index::{DefKind, Index, Role};
use crate::project::Loaded;
use crate::syntax::Span;

pub struct Hover {
    pub span: Span,
    pub markdown: String,
}

pub fn hover(
    index: &Index,
    loaded: &Loaded,
    file: usize,
    offset: usize,
    line_of: impl Fn(usize, usize) -> usize,
) -> Option<Hover> {
    let id = index.resolve(file, offset)?;
    let def = &index.defs[id];
    let covers = |span: Span| span.start <= offset && offset <= span.end;
    let span = if def.file == file && covers(def.name_span) {
        def.name_span
    } else {
        index.references(id).find(|r| r.file == file && covers(r.span))?.span
    };

    let source = &loaded.sources[def.file];
    let count = |role: Role| index.references(id).filter(|r| r.role == role).count();
    let mut markdown = String::new();
    match def.kind {
        DefKind::Variable => {
            let line = line_of(def.file, def.name_span.start) + 1;
            let name = &def.name;
            match &def.ty {
                Some(ty) => markdown.push_str(&format!("```isle\n{name}: {ty}\n```\n")),
                None => markdown.push_str(&format!("```isle\n{name}\n```\n")),
            }
            markdown.push_str(&format!("variable bound on line {line}, {} uses", index.references(id).count()));
        }
        DefKind::Variant => {
            markdown.push_str(&format!("```isle\n{}\n```\n", collapse(def.form.text(&source.text))));
            markdown.push_str(&format!("variant, {} uses", index.references(id).count()));
        }
        kind => {
            markdown.push_str(&format!("```isle\n{}\n```\n", collapse(def.form.text(&source.text))));
            if let Some(doc) = doc_comment(&source.text, def.form.start) {
                markdown.push_str(&format!("{doc}\n\n"));
            }
            if kind == DefKind::Term {
                let mut kinds: Vec<&str> = Vec::new();
                if count(Role::RuleHead) > 0 {
                    kinds.push("internal constructor");
                }
                for imp in &def.impls {
                    if !kinds.contains(imp) {
                        kinds.push(imp);
                    }
                }
                let kinds = if kinds.is_empty() { "no implementation".to_owned() } else { kinds.join(", ") };
                markdown.push_str(&format!(
                    "{kinds}  \n{} rules define it, {} pattern matches, {} calls",
                    count(Role::RuleHead),
                    count(Role::Match),
                    count(Role::Call)
                ));
            } else {
                markdown.push_str(&format!("{}, {} uses", kind.label(), index.references(id).count()));
            }
        }
    }
    Some(Hover { span, markdown })
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn doc_comment(text: &str, form_start: usize) -> Option<String> {
    let line_start = text[..form_start].rfind('\n').map_or(0, |newline| newline + 1);
    if !text[line_start..form_start].trim().is_empty() {
        return None;
    }
    let mut lines: Vec<&str> = text[..line_start]
        .lines()
        .rev()
        .map_while(|line| line.trim_start().strip_prefix(";;"))
        .map(str::trim)
        .collect();
    lines.reverse();
    (!lines.is_empty()).then(|| lines.join("\n"))
}
