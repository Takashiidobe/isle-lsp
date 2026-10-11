use crate::index::{DefKind, Index};
use crate::project::Loaded;

pub struct Hint {
    pub offset: usize,
    pub label: String,
}

pub fn hints(index: &Index, loaded: &Loaded, file: usize) -> Vec<Hint> {
    let text = &loaded.sources[file].text;
    let mut found: Vec<Hint> = index
        .defs
        .iter()
        .filter(|def| def.file == file && def.kind == DefKind::Variable)
        .filter_map(|def| {
            let ty = def.ty.as_ref()?;
            let written = text[def.name_span.end..].trim_start().starts_with(ty.as_str());
            (!written).then(|| Hint { offset: def.name_span.end, label: format!(": {ty}") })
        })
        .collect();
    found.sort_by_key(|hint| hint.offset);
    found
}
