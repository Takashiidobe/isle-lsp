use std::collections::BTreeMap;

use crate::index::{DefId, DefKind, Index, Role};
use crate::navigation::Place;
use crate::project::Loaded;
use crate::rules;
use crate::syntax::Span;

pub struct Call {
    pub def: DefId,
    pub sites: Vec<Place>,
}

pub fn prepare(index: &Index, file: usize, offset: usize) -> Option<DefId> {
    let id = index.resolve(file, offset)?;
    callable(index, id).then_some(id)
}

pub fn incoming(index: &Index, loaded: &Loaded, def: DefId) -> Vec<Call> {
    let owners = Owners::build(index, loaded);
    let mut found = BTreeMap::new();
    for reference in index.references(def).filter(|r| r.role == Role::Call) {
        if let Some(caller) = owners.of(reference.file, reference.span) {
            found.entry(caller).or_insert_with(Vec::new).push(Place { file: reference.file, span: reference.span });
        }
    }
    collect(found)
}

pub fn outgoing(index: &Index, loaded: &Loaded, def: DefId) -> Vec<Call> {
    let owners = Owners::build(index, loaded);
    let mut found = BTreeMap::new();
    for reference in index.refs.iter().filter(|r| r.role == Role::Call) {
        let Some(callee) = reference.def.filter(|&callee| callable(index, callee)) else { continue };
        if owners.of(reference.file, reference.span) == Some(def) {
            found.entry(callee).or_insert_with(Vec::new).push(Place { file: reference.file, span: reference.span });
        }
    }
    collect(found)
}

fn callable(index: &Index, id: DefId) -> bool {
    matches!(index.defs[id].kind, DefKind::Term | DefKind::Macro)
}

fn collect(found: BTreeMap<DefId, Vec<Place>>) -> Vec<Call> {
    found.into_iter().map(|(def, sites)| Call { def, sites }).collect()
}

struct Owners {
    forms: Vec<(usize, Span, DefId)>,
}

impl Owners {
    fn build(index: &Index, loaded: &Loaded) -> Owners {
        let heads: BTreeMap<(usize, usize), DefId> = index
            .refs
            .iter()
            .filter(|r| r.role == Role::RuleHead)
            .filter_map(|r| Some(((r.file, r.span.start), r.def?)))
            .collect();
        let mut forms: Vec<(usize, Span, DefId)> = rules::rules(loaded)
            .iter()
            .filter_map(|rule| Some((rule.file, rule.form, *heads.get(&(rule.file, rule.head_span.start))?)))
            .collect();
        for (id, def) in index.defs.iter().enumerate() {
            if def.kind == DefKind::Macro {
                forms.push((def.file, def.form, id));
            }
        }
        forms.sort_by_key(|(file, span, _)| (*file, span.start));
        Owners { forms }
    }

    fn of(&self, file: usize, span: Span) -> Option<DefId> {
        let after = self.forms.partition_point(|(f, form, _)| (*f, form.start) <= (file, span.start));
        let (f, form, owner) = self.forms.get(after.checked_sub(1)?)?;
        (*f == file && span.end <= form.end).then_some(*owner)
    }
}
