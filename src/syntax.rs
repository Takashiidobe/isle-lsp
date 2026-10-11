#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub span: Span,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomKind {
    Symbol,
    Int,
    At,
}

#[derive(Clone, Debug)]
pub struct Atom {
    pub kind: AtomKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct List {
    pub span: Span,
    pub items: Vec<Node>,
    pub closed: bool,
}

#[derive(Clone, Debug)]
pub enum Node {
    List(List),
    Atom(Atom),
}

impl Node {
    pub fn span(&self) -> Span {
        match self {
            Node::List(list) => list.span,
            Node::Atom(atom) => atom.span,
        }
    }
}

impl List {
    pub fn head(&self) -> Option<&Atom> {
        match self.items.first() {
            Some(Node::Atom(atom)) if atom.kind == AtomKind::Symbol => Some(atom),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKind {
    Pragma,
    Type,
    Decl,
    Attr,
    Spec,
    Macro,
    State,
    Model,
    Form,
    Instantiate,
    Rule,
    Extractor,
    Extern,
    Convert,
}

pub const FORMS: [(&str, FormKind); 14] = [
    ("pragma", FormKind::Pragma),
    ("type", FormKind::Type),
    ("decl", FormKind::Decl),
    ("attr", FormKind::Attr),
    ("spec", FormKind::Spec),
    ("macro", FormKind::Macro),
    ("state", FormKind::State),
    ("model", FormKind::Model),
    ("form", FormKind::Form),
    ("instantiate", FormKind::Instantiate),
    ("rule", FormKind::Rule),
    ("extractor", FormKind::Extractor),
    ("extern", FormKind::Extern),
    ("convert", FormKind::Convert),
];

impl FormKind {
    pub fn from_keyword(word: &str) -> Option<FormKind> {
        FORMS.iter().find(|(name, _)| *name == word).map(|&(_, kind)| kind)
    }

    pub fn keyword(self) -> &'static str {
        FORMS.iter().find(|&&(_, kind)| kind == self).map(|&(name, _)| name).unwrap()
    }
}

pub fn form_kind(source: &str, list: &List) -> Option<FormKind> {
    FormKind::from_keyword(list.head()?.span.text(source))
}

#[derive(Debug, Default)]
pub struct File {
    pub items: Vec<Node>,
    pub comments: Vec<Span>,
    pub errors: Vec<SyntaxError>,
}
