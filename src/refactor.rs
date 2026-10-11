use crate::index::{DefKind, Index, Role};
use crate::navigation::{Edit, Place};
use crate::project::Loaded;
use crate::rules;
use crate::syntax::{AtomKind, List, Node, Span};

pub struct Refactor {
    pub title: String,
    pub edits: Vec<Edit>,
}

pub fn all(loaded: &Loaded, index: &Index, file: usize, start: usize, end: usize) -> Vec<Refactor> {
    [
        toggle_rule_name(loaded, index, file, start),
        inline_term(loaded, index, file, start),
        extract_term(loaded, index, file, start, end),
        simplify_template(loaded, index, file, start),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn insert(file: usize, at: usize, new_text: String) -> Edit {
    Edit { place: Place { file, span: Span { start: at, end: at } }, new_text }
}

fn replace(file: usize, span: Span, new_text: String) -> Edit {
    Edit { place: Place { file, span }, new_text }
}

fn toggle_rule_name(loaded: &Loaded, index: &Index, file: usize, offset: usize) -> Option<Refactor> {
    let text = &loaded.sources[file].text;
    let rule = rules::rules(loaded).into_iter().find(|r| r.file == file && r.form.start <= offset && offset <= r.form.end)?;
    if let Some(name) = rule.name {
        let tail = &text[name.end..];
        let gap = tail.len() - tail.trim_start().len();
        let span = Span { start: name.start, end: name.end + gap };
        return Some(Refactor { title: format!("Remove name of rule `{}`", name.text(text)), edits: vec![replace(file, span, String::new())] });
    }
    let first = rule.list.items.get(1)?.span().start;
    let taken = |candidate: &str| index.defs.iter().any(|def| def.kind == DefKind::Rule && def.name == candidate);
    let name = (1..).map(|n| format!("{}_{n}", rule.head)).find(|candidate| !taken(candidate))?;
    Some(Refactor { title: format!("Name rule `{name}`"), edits: vec![insert(file, first, format!("{name} "))] })
}

fn find_call(nodes: &[Node], head_start: usize) -> Option<&List> {
    for node in nodes {
        let Node::List(list) = node else { continue };
        if list.head().is_some_and(|head| head.span.start == head_start) {
            return Some(list);
        }
        if list.span.start <= head_start && head_start < list.span.end {
            return find_call(&list.items, head_start);
        }
    }
    None
}

fn contains_let(text: &str, node: &Node) -> bool {
    match node {
        Node::Atom(_) => false,
        Node::List(list) => list.head().is_some_and(|head| head.span.text(text) == "let") || list.items.iter().any(|item| contains_let(text, item)),
    }
}

fn substitute(text: &str, node: &Node, bindings: &[(&str, &str)]) -> String {
    match node {
        Node::Atom(atom) => {
            let word = atom.span.text(text);
            let replacement = bindings.iter().find(|(param, _)| atom.kind == AtomKind::Symbol && *param == word);
            replacement.map_or(word, |(_, arg)| arg).to_owned()
        }
        Node::List(list) => {
            let parts: Vec<String> = list.items.iter().map(|item| substitute(text, item, bindings)).collect();
            format!("({})", parts.join(" "))
        }
    }
}

fn inline_term(loaded: &Loaded, index: &Index, file: usize, offset: usize) -> Option<Refactor> {
    let reference = index.refs.iter().find(|r| r.file == file && r.role == Role::Call && r.span.start <= offset && offset <= r.span.end)?;
    let def = &index.defs[reference.def?];
    if def.kind != DefKind::Term || !def.impls.is_empty() {
        return None;
    }
    let all = rules::rules(loaded);
    let mut own = all.iter().filter(|rule| rule.head == def.name);
    let rule = own.next().filter(|_| own.next().is_none())?;
    let params: Vec<&str> = rule.args.iter().map(|arg| plain_variable(rule.text, arg)).collect::<Option<_>>()?;
    let distinct = params.iter().enumerate().all(|(at, param)| !params[..at].contains(param));
    let source = &loaded.sources[file];
    let call = find_call(&source.file.items, reference.span.start)?;
    let args: Vec<&str> = call.items.iter().skip(1).map(|arg| arg.span().text(&source.text)).collect();
    if !rule.clauses.is_empty() || !distinct || params.len() != args.len() || !call.closed || contains_let(rule.text, rule.rhs) {
        return None;
    }
    let bindings: Vec<(&str, &str)> = params.into_iter().zip(args).collect();
    let new_text = substitute(rule.text, rule.rhs, &bindings);
    Some(Refactor { title: format!("Inline `{}`", def.name), edits: vec![replace(file, call.span, new_text)] })
}

fn plain_variable<'a>(text: &'a str, node: &Node) -> Option<&'a str> {
    let Node::Atom(atom) = node else { return None };
    let word = atom.span.text(text);
    let plain = atom.kind == AtomKind::Symbol && word != "_" && !word.starts_with(['$', '#']) && !word.contains('.');
    plain.then_some(word)
}

fn find_exact(nodes: &[Node], span: Span) -> Option<&List> {
    for node in nodes {
        let Node::List(list) = node else { continue };
        if list.span == span {
            return Some(list);
        }
        if list.span.start <= span.start && span.end <= list.span.end {
            return find_exact(&list.items, span);
        }
    }
    None
}

fn extract_term(loaded: &Loaded, index: &Index, file: usize, start: usize, end: usize) -> Option<Refactor> {
    if start >= end {
        return None;
    }
    let source = &loaded.sources[file];
    let rule = rules::rules(loaded).into_iter().find(|r| r.file == file && r.rhs.span().start <= start && end <= r.rhs.span().end)?;
    let selected = find_exact(std::slice::from_ref(rule.rhs), Span { start, end })?;
    if !selected.closed {
        return None;
    }
    let inside = |span: Span| start <= span.start && span.end <= end;
    let head = selected.head()?.span;
    let result = index.refs.iter().find(|r| r.file == file && r.role == Role::Call && r.span == head)?.def.and_then(|id| index.defs[id].ret.as_deref())?;
    let mut params: Vec<(&str, &str)> = Vec::new();
    for reference in index.refs.iter().filter(|r| r.file == file && r.role == Role::Variable && inside(r.span)) {
        let Some(def) = reference.def.map(|id| &index.defs[id]) else { continue };
        let bound_inside = def.file == file && inside(def.name_span);
        if !bound_inside && !params.iter().any(|(name, _)| *name == def.name) {
            params.push((&def.name, def.ty.as_deref()?));
        }
    }
    let taken = |candidate: &str| index.defs.iter().any(|def| def.kind == DefKind::Term && def.name == candidate);
    let name = (1..).map(|n| format!("extracted_{n}")).find(|candidate| !taken(candidate))?;
    let names: Vec<&str> = params.iter().map(|(name, _)| *name).collect();
    let types: Vec<&str> = params.iter().map(|(_, ty)| *ty).collect();
    let call = format!("({})", [name.as_str()].into_iter().chain(names).collect::<Vec<_>>().join(" "));
    let definition = format!(
        "(decl {name} ({}) {result})\n(rule {call} {})\n\n",
        types.join(" "),
        selected.span.text(&source.text)
    );
    Some(Refactor {
        title: format!("Extract into term `{name}`"),
        edits: vec![insert(file, rule.form.start, definition), replace(file, selected.span, call)],
    })
}

fn simplify_template(loaded: &Loaded, index: &Index, file: usize, offset: usize) -> Option<Refactor> {
    let id = index.resolve(file, offset)?;
    let def = &index.defs[id];
    let bundled = loaded.sources[def.file].path.file_stem().is_some_and(|stem| stem == "clif_opt");
    if def.kind != DefKind::Term || !bundled || !index.defs.iter().any(|d| d.kind == DefKind::Term && d.name == "simplify") {
        return None;
    }
    let mut values = ["x", "y", "z", "a", "b", "c"].into_iter();
    let mut first_value = None;
    let mut names: Vec<String> = Vec::new();
    for ty in &def.args {
        let name = match ty.as_str() {
            "Type" => "ty".to_owned(),
            "Value" => values.next()?.to_owned(),
            other => other.to_lowercase(),
        };
        let name = if names.contains(&name) { format!("{name}{}", names.len()) } else { name };
        if ty == "Value" {
            first_value.get_or_insert_with(|| name.clone());
        }
        names.push(name);
    }
    let result = first_value?;
    let text = &loaded.sources[file].text;
    let separator = if text.ends_with('\n') { "\n" } else { "\n\n" };
    let pattern = format!("({} {})", def.name, names.join(" "));
    let rule = format!("{separator}(rule (simplify {pattern}) {result})\n");
    Some(Refactor { title: format!("Insert simplify rule for `{}`", def.name), edits: vec![insert(file, text.len(), rule)] })
}
