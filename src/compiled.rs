use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;

use cranelift_isle::error::Errors;
use cranelift_isle::files::Files;
use cranelift_isle::lexer::Lexer;
use cranelift_isle::sema::{TermEnv, TermId, Type, TypeEnv, TypeId, VariantId};
use cranelift_isle::trie_again::RuleSet;
use cranelift_isle::{overlap, parser as isle_parser, recursion};

use crate::project::Loaded;

pub struct Compiled {
    pub files: Arc<Files>,
    pub types: TypeEnv,
    pub terms: TermEnv,
    pub rule_sets: Vec<(TermId, RuleSet)>,
}

pub fn compile(loaded: &Loaded, name: impl Fn(&Path) -> String) -> Result<Compiled, String> {
    let files = Arc::new(Files::from_names_and_contents(
        loaded.sources.iter().map(|source| (name(&source.path), source.text.clone())),
    ));
    panic::catch_unwind(AssertUnwindSafe(|| build(loaded, files)))
        .unwrap_or_else(|_| Err("internal error in cranelift-isle while compiling this project".to_owned()))
}

fn build(loaded: &Loaded, files: Arc<Files>) -> Result<Compiled, String> {
    let fail = |errors| Errors::new(errors, files.clone()).to_string();
    let mut defs = Vec::new();
    for (idx, source) in loaded.sources.iter().enumerate() {
        let parsed = Lexer::new(idx, &source.text).and_then(isle_parser::parse).map_err(|error| fail(vec![error]))?;
        defs.extend(parsed);
    }
    let mut types = TypeEnv::from_ast(&defs).map_err(fail)?;
    let terms = TermEnv::from_ast(&mut types, &defs, true).map_err(fail)?;
    let rule_sets = overlap::check(&terms).map_err(fail)?;
    recursion::check(&rule_sets, &terms).map_err(fail)?;
    Ok(Compiled { files, types, terms, rule_sets })
}

impl Compiled {
    pub fn term_name(&self, term: TermId) -> &str {
        &self.types.syms[self.terms.terms[term.index()].name.index()]
    }

    pub fn type_name(&self, ty: TypeId) -> String {
        self.types.types[ty.index()].name(&self.types).to_owned()
    }

    pub fn variant_name(&self, ty: TypeId, variant: VariantId) -> String {
        match &self.types.types[ty.index()] {
            Type::Enum { variants, .. } => self.types.syms[variants[variant.index()].fullname.index()].clone(),
            _ => self.type_name(ty),
        }
    }

    pub fn find(&self, name: &str) -> Option<(TermId, &RuleSet)> {
        self.rule_sets.iter().find(|(term, _)| self.term_name(*term) == name).map(|(term, set)| (*term, set))
    }
}
