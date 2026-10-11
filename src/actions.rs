use std::collections::HashMap;

use crate::index::{DefKind, Index};
use crate::navigation::Place;
use crate::project::Loaded;
use crate::syntax::Span;
use tower_lsp_server::ls_types::{
    CodeAction, CodeActionKind, Diagnostic, Range, TextEdit, Uri, WorkspaceEdit,
};

pub fn unused_decl(index: &Index, loaded: &Loaded, file: usize, offset: usize) -> Option<(String, Place)> {
    let (id, def) = index
        .defs
        .iter()
        .enumerate()
        .find(|(_, def)| def.kind == DefKind::Term && def.file == file && def.form.start <= offset && offset <= def.form.end)?;
    if index.references(id).next().is_some() {
        return None;
    }
    let text = &loaded.sources[file].text;
    let mut end = def.form.end;
    if text[end..].starts_with('\n') {
        end += 1;
    }
    Some((format!("Remove unused declaration of `{}`", def.name), Place { file, span: Span { start: def.form.start, end } }))
}

pub fn quick_fixes(uri: &Uri, diagnostics: &[Diagnostic]) -> Vec<CodeAction> {
    let mut actions = Vec::new();
    for diagnostic in diagnostics {
        let Some(fixes) = diagnostic.data.as_ref().and_then(|data| data.get("fixes")).and_then(|fixes| fixes.as_array())
        else {
            continue;
        };
        for fix in fixes {
            let (Some(title), Some(new_text), Some(range)) = (
                fix.get("title").and_then(|title| title.as_str()),
                fix.get("newText").and_then(|text| text.as_str()),
                fix.get("range").and_then(|range| serde_json::from_value::<Range>(range.clone()).ok()),
            ) else {
                continue;
            };
            actions.push(CodeAction {
                title: title.to_owned(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(HashMap::from([(uri.clone(), vec![TextEdit { range, new_text: new_text.to_owned() }])])),
                    ..WorkspaceEdit::default()
                }),
                ..CodeAction::default()
            });
        }
    }
    actions
}
