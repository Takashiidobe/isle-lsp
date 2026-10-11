use crate::index::{DefId, DefKind, Index};
use crate::project::Loaded;
use crate::syntax::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub file: usize,
    pub span: Span,
}

#[derive(Debug)]
pub struct Edit {
    pub place: Place,
    pub new_text: String,
}

pub fn definition(index: &Index, file: usize, offset: usize) -> Option<Place> {
    let def = &index.defs[index.resolve(file, offset)?];
    Some(Place { file: def.file, span: def.name_span })
}

pub fn references(index: &Index, file: usize, offset: usize, include_declaration: bool) -> Vec<Place> {
    let Some(id) = index.resolve(file, offset) else { return Vec::new() };
    let def = &index.defs[id];
    let mut found = Vec::new();
    if include_declaration {
        found.push(Place { file: def.file, span: def.name_span });
    }
    found.extend(index.references(id).map(|r| Place { file: r.file, span: r.span }));
    found
}

pub fn highlights(index: &Index, file: usize, offset: usize) -> Vec<Place> {
    let mut found = references(index, file, offset, true);
    found.retain(|place| place.file == file);
    found
}

pub fn prepare_rename(index: &Index, loaded: &Loaded, file: usize, offset: usize) -> Option<Place> {
    let id = index.resolve(file, offset)?;
    let def = &index.defs[id];
    if def.file == file && covers(def.name_span, offset) {
        return Some(Place { file, span: def.name_span });
    }
    let reference = index.references(id).find(|r| r.file == file && covers(r.span, offset))?;
    Some(Place { file, span: renamed_part(loaded, index, id, reference.file, reference.span) })
}

pub fn rename(index: &Index, loaded: &Loaded, file: usize, offset: usize, new_name: &str) -> Result<Vec<Edit>, String> {
    let id = index.resolve(file, offset).ok_or("nothing to rename here")?;
    let def = &index.defs[id];
    validate(index, loaded, id, new_name)?;

    let mut edits = vec![Edit { place: Place { file: def.file, span: def.name_span }, new_text: new_name.to_owned() }];
    for reference in index.references(id) {
        let span = renamed_part(loaded, index, id, reference.file, reference.span);
        edits.push(Edit { place: Place { file: reference.file, span }, new_text: new_name.to_owned() });
    }
    if def.kind == DefKind::Type {
        let prefix = format!("{}.", def.name);
        for (variant, other) in index.defs.iter().enumerate() {
            if other.kind != DefKind::Variant || !other.name.starts_with(&prefix) {
                continue;
            }
            for reference in index.references(variant) {
                let start = reference.span.start;
                let span = Span { start, end: start + def.name.len() };
                edits.push(Edit { place: Place { file: reference.file, span }, new_text: new_name.to_owned() });
            }
        }
    }
    edits.sort_by_key(|edit| (edit.place.file, edit.place.span.start));
    Ok(edits)
}

fn covers(span: Span, offset: usize) -> bool {
    span.start <= offset && offset <= span.end
}

fn renamed_part(loaded: &Loaded, index: &Index, id: DefId, file: usize, span: Span) -> Span {
    let def = &index.defs[id];
    if def.kind != DefKind::Variant {
        return span;
    }
    let text = span.text(&loaded.sources[file].text);
    let tail = text.rsplit('.').next().unwrap_or(text).len();
    Span { start: span.end - tail, end: span.end }
}

fn validate(index: &Index, loaded: &Loaded, id: DefId, new_name: &str) -> Result<(), String> {
    let def = &index.defs[id];
    let malformed = new_name.is_empty()
        || new_name.chars().any(|c| c.is_whitespace() || matches!(c, '(' | ')' | ';' | '@'))
        || new_name.trim_start_matches('-').starts_with(|c: char| c.is_ascii_digit());
    if malformed {
        return Err(format!("`{new_name}` is not a valid ISLE symbol"));
    }
    match def.kind {
        DefKind::Const if !new_name.starts_with('$') => return Err("constant names must start with `$`".to_owned()),
        DefKind::Const => {}
        _ if new_name.starts_with('$') || new_name.starts_with('#') => {
            return Err(format!("{} names cannot start with `{}`", def.kind.label(), &new_name[..1]));
        }
        DefKind::Variant if new_name.contains('.') => return Err("variant names cannot contain `.`".to_owned()),
        DefKind::Variable if new_name == "_" => return Err("`_` is the wildcard pattern, not a variable name".to_owned()),
        _ => {}
    }

    let full_name = match def.kind {
        DefKind::Variant => {
            let ty = def.name.rsplit_once('.').map_or("", |(ty, _)| ty);
            format!("{ty}.{new_name}")
        }
        _ => new_name.to_owned(),
    };
    if full_name == def.name {
        return Err("the new name is the same as the old one".to_owned());
    }
    let is_global = |kind: DefKind| !matches!(kind, DefKind::Variable);
    let form_of = |file: usize, span: Span| {
        loaded.sources[file].file.items.iter().map(|node| node.span()).find(|form| form.start <= span.start && span.start < form.end)
    };
    for (other_id, other) in index.defs.iter().enumerate() {
        if other_id == id || other.name != full_name {
            continue;
        }
        let clashes = if def.kind == DefKind::Variable {
            !is_global(other.kind) && other.file == def.file && form_of(def.file, def.name_span) == form_of(other.file, other.name_span)
                || matches!(other.kind, DefKind::Variant)
        } else {
            is_global(other.kind)
        };
        if clashes {
            let kind = if other.kind == def.kind { "already exists" } else { "clashes with a different kind of definition" };
            return Err(format!("`{full_name}` {kind}: {} {}", other.kind.label(), other.name));
        }
    }
    Ok(())
}
