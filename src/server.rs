use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_lsp_server::jsonrpc::{Error, Result};
use tower_lsp_server::ls_types::{
    CodeAction, CodeActionKind, CreateFile, CreateFileOptions, DocumentChangeOperation, DocumentChanges, OptionalVersionedTextDocumentIdentifier, ResourceOp, TextDocumentEdit, CodeLens, CodeLensOptions, CodeLensParams, Command, TextDocumentIdentifier, CodeActionOrCommand, CodeActionParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    FoldingRange, FoldingRangeParams, FoldingRangeProviderCapability, SemanticToken, SemanticTokenModifier,
    SemanticTokenType, SemanticTokens, SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions,
    SemanticTokensParams, SemanticTokensResult, SemanticTokensServerCapabilities, SymbolKind, WorkspaceSymbol,
    WorkspaceSymbolParams, WorkspaceSymbolResponse, InlayHint, InlayHintKind, InlayHintLabel, InlayHintParams, CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams,
    CompletionResponse, InsertTextFormat, ParameterInformation, ParameterLabel, SignatureHelp, SignatureHelpOptions,
    SignatureHelpParams, SignatureInformation, CodeActionProviderCapability, CodeActionResponse, Diagnostic,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentHighlight,
    DocumentHighlightKind, DocumentHighlightParams, GotoDefinitionParams, GotoDefinitionResponse, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult, Location, MarkupContent,
    MarkupKind, OneOf, Position, PrepareRenameResponse, Range, ReferenceParams, RenameOptions,
    RenameParams, ServerCapabilities, ServerInfo, TextDocumentPositionParams, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Uri, WorkDoneProgressOptions, WorkspaceEdit,
};
use tower_lsp_server::{Client, ClientSocket, LanguageServer, LspService};

use crate::actions;
use crate::completion::{self, ItemKind};
use crate::diagnostics;
use crate::document::{Document, position_in};
use crate::filetest;
use crate::hover;
use crate::index::{DefKind, Index};
use crate::rules;
use crate::inlay;
use crate::lints;
use crate::navigation::{self, Place};
use crate::project::{self, Loaded, Project};
use crate::semantic;
use crate::symbols::{self, SymKind};
use crate::syntax::Span;
use crate::view;

pub fn service() -> (LspService<Backend>, ClientSocket) {
    LspService::build(Backend::new)
        .custom_method("isle/rulesForTerm", Backend::rules_for_term)
        .custom_method("isle/rulesForClif", Backend::rules_for_clif)
        .custom_method("isle/view", Backend::view)
        .finish()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewParams {
    text_document: TextDocumentIdentifier,
    view: String,
    term: Option<String>,
    position: Option<Position>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesForTermParams {
    text_document: TextDocumentIdentifier,
    position: Position,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesForClifParams {
    text_document: TextDocumentIdentifier,
    snippet: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedRule {
    location: Location,
    priority: i64,
    wins_over: Vec<Location>,
}

fn ranked_rules(analysis: &Analysis, ranked: Vec<rules::Ranked>) -> Vec<RankedRule> {
    let place = |file: usize, span: Span| analysis.location(Place { file, span });
    ranked
        .into_iter()
        .filter_map(|rule| {
            Some(RankedRule {
                location: place(rule.file, rule.span)?,
                priority: rule.priority,
                wins_over: rule.wins_over.into_iter().filter_map(|(file, span)| place(file, span)).collect(),
            })
        })
        .collect()
}

fn common_dir(loaded: &Loaded) -> PathBuf {
    let mut dirs = loaded.sources.iter().filter_map(|source| source.path.parent());
    let Some(first) = dirs.next() else { return PathBuf::new() };
    dirs.fold(first.to_path_buf(), |common, dir| common.ancestors().find(|a| dir.starts_with(a)).unwrap_or(&common).to_path_buf())
}

fn symbol_kind(kind: SymKind) -> SymbolKind {
    match kind {
        SymKind::Type => SymbolKind::CLASS,
        SymKind::Variant => SymbolKind::ENUM_MEMBER,
        SymKind::Term | SymKind::Group => SymbolKind::FUNCTION,
        SymKind::Const => SymbolKind::CONSTANT,
        SymKind::Macro => SymbolKind::OPERATOR,
        SymKind::Rule => SymbolKind::METHOD,
    }
}

#[allow(deprecated)]
fn document_symbol(text: &str, symbol: &symbols::Symbol) -> DocumentSymbol {
    let range = |span: Span| Range::new(position_in(text, span.start), position_in(text, span.end));
    DocumentSymbol {
        name: symbol.name.clone(),
        detail: (!symbol.detail.is_empty()).then(|| symbol.detail.clone()),
        kind: symbol_kind(symbol.kind),
        tags: None,
        deprecated: None,
        range: range(symbol.range),
        selection_range: range(symbol.selection),
        children: (!symbol.children.is_empty()).then(|| symbol.children.iter().map(|child| document_symbol(text, child)).collect()),
    }
}

struct Analysis {
    loaded: Loaded,
    index: Index,
    file: usize,
    offset: usize,
}

impl Analysis {
    fn location(&self, place: Place) -> Option<Location> {
        let source = &self.loaded.sources[place.file];
        Some(Location::new(Uri::from_file_path(&source.path)?, self.range(place)))
    }

    fn range(&self, place: Place) -> Range {
        let text = &self.loaded.sources[place.file].text;
        Range::new(position_in(text, place.span.start), position_in(text, place.span.end))
    }
}

pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Uri, Document>>>,
    projects: Arc<RwLock<HashMap<Uri, Project>>>,
    published: Arc<RwLock<HashSet<Uri>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self { client, documents: Arc::default(), projects: Arc::default(), published: Arc::default() }
    }

    async fn analyze(&self, uri: &Uri, position: Position) -> Option<Analysis> {
        let documents = self.documents.read().await;
        let projects = self.projects.read().await;
        let open: HashMap<PathBuf, String> = documents
            .iter()
            .filter_map(|(uri, document)| Some((project::normalize(&uri.to_file_path()?), document.text().to_owned())))
            .collect();
        let loaded = projects.get(uri)?.load(|path| open.get(path).cloned());
        let here = project::normalize(&uri.to_file_path()?);
        let file = loaded.sources.iter().position(|source| source.path == here)?;
        let offset = documents.get(uri)?.offset(position);
        let index = Index::build(&loaded);
        Some(Analysis { loaded, index, file, offset })
    }

    async fn rules_for_term(&self, params: RulesForTermParams) -> Result<Vec<RankedRule>> {
        let Some(analysis) = self.analyze(&params.text_document.uri, params.position).await else { return Ok(Vec::new()) };
        let ranked = analysis
            .index
            .resolve(analysis.file, analysis.offset)
            .filter(|&id| analysis.index.defs[id].kind == DefKind::Term)
            .map(|id| rules::for_term(&analysis.loaded, &analysis.index.defs[id].name))
            .unwrap_or_default();
        Ok(ranked_rules(&analysis, ranked))
    }

    async fn view(&self, params: ViewParams) -> Result<view::View> {
        let position = params.position.unwrap_or(Position::new(0, 0));
        let analysis = self
            .analyze(&params.text_document.uri, position)
            .await
            .ok_or_else(|| Error::invalid_params("the document is not open"))?;
        let under_cursor = params.position.and_then(|_| {
            analysis
                .index
                .resolve(analysis.file, analysis.offset)
                .filter(|&id| analysis.index.defs[id].kind == DefKind::Term)
                .map(|id| analysis.index.defs[id].name.clone())
        });
        let term = params.term.or(under_cursor);
        let base = common_dir(&analysis.loaded);
        let name = |path: &std::path::Path| path.strip_prefix(&base).unwrap_or(path).display().to_string();
        view::render(&analysis.loaded, name, &params.view, term.as_deref()).map_err(Error::invalid_params)
    }

    async fn rules_for_clif(&self, params: RulesForClifParams) -> Result<Vec<RankedRule>> {
        let Some(analysis) = self.analyze(&params.text_document.uri, Position::new(0, 0)).await else { return Ok(Vec::new()) };
        let ranked = rules::for_clif(&analysis.loaded, &analysis.index, &params.snippet);
        Ok(ranked_rules(&analysis, ranked))
    }

    async fn refresh_diagnostics(&self) {
        let mut found: HashMap<Uri, Vec<Diagnostic>> = HashMap::new();
        {
            let projects = self.projects.read().await;
            let documents = self.documents.read().await;
            let open: HashMap<PathBuf, String> = documents
                .iter()
                .filter_map(|(uri, document)| Some((project::normalize(&uri.to_file_path()?), document.text().to_owned())))
                .collect();
            let mut analyzed: Vec<&Vec<PathBuf>> = Vec::new();
            for project in projects.values() {
                if analyzed.contains(&&project.files) {
                    continue;
                }
                analyzed.push(&project.files);
                let loaded = project.load(|path| open.get(path).cloned());
                for file in diagnostics::all(project, &loaded) {
                    if let Some(uri) = Uri::from_file_path(&file.path) {
                        found.insert(uri, file.diagnostics);
                    }
                }
            }
        }

        let mut published = self.published.write().await;
        for stale in published.iter().filter(|uri| !found.contains_key(*uri)) {
            self.client.publish_diagnostics(stale.clone(), Vec::new(), None).await;
        }
        *published = found.keys().cloned().collect();
        for (uri, diagnostics) in found {
            self.client.publish_diagnostics(uri, diagnostics, None).await;
        }
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::INCREMENTAL)),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                code_lens_provider: Some(CodeLensOptions { resolve_provider: Some(false) }),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["(".to_owned(), " ".to_owned()]),
                    ..CompletionOptions::default()
                }),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec![" ".to_owned()]),
                    ..SignatureHelpOptions::default()
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(SemanticTokensOptions {
                    legend: SemanticTokensLegend {
                        token_types: semantic::TYPES.iter().map(|&name| SemanticTokenType::new(name)).collect(),
                        token_modifiers: semantic::MODIFIERS.iter().map(|&name| SemanticTokenModifier::new(name)).collect(),
                    },
                    range: None,
                    full: Some(SemanticTokensFullOptions::Bool(true)),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                inlay_hint_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo { name: "isle-lsp".into(), version: Some(env!("CARGO_PKG_VERSION").into()) }),
            offset_encoding: None,
        })
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn goto_definition(&self, params: GotoDefinitionParams) -> Result<Option<GotoDefinitionResponse>> {
        let at = params.text_document_position_params;
        let Some(analysis) = self.analyze(&at.text_document.uri, at.position).await else { return Ok(None) };
        let place = navigation::definition(&analysis.index, analysis.file, analysis.offset);
        Ok(place.and_then(|place| analysis.location(place)).map(GotoDefinitionResponse::Scalar))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let at = params.text_document_position_params;
        let Some(analysis) = self.analyze(&at.text_document.uri, at.position).await else { return Ok(None) };
        let found = hover::hover(&analysis.index, &analysis.loaded, analysis.file, analysis.offset, |file, offset| {
            analysis.loaded.sources[file].text[..offset].matches('\n').count()
        });
        Ok(found.map(|found| Hover {
            contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: found.markdown }),
            range: Some(analysis.range(Place { file: analysis.file, span: found.span })),
        }))
    }

    async fn document_symbol(&self, params: DocumentSymbolParams) -> Result<Option<DocumentSymbolResponse>> {
        let Some(analysis) = self.analyze(&params.text_document.uri, Position::new(0, 0)).await else { return Ok(None) };
        let symbols = symbols::document_symbols(&analysis.index, &analysis.loaded, analysis.file);
        let text = &analysis.loaded.sources[analysis.file].text;
        Ok(Some(DocumentSymbolResponse::Nested(symbols.iter().map(|symbol| document_symbol(text, symbol)).collect())))
    }

    async fn symbol(&self, params: WorkspaceSymbolParams) -> Result<Option<WorkspaceSymbolResponse>> {
        let documents = self.documents.read().await;
        let projects = self.projects.read().await;
        let open: HashMap<PathBuf, String> = documents
            .iter()
            .filter_map(|(uri, document)| Some((project::normalize(&uri.to_file_path()?), document.text().to_owned())))
            .collect();
        let mut seen: Vec<&Vec<PathBuf>> = Vec::new();
        let mut found = Vec::new();
        for project in projects.values() {
            if seen.contains(&&project.files) {
                continue;
            }
            seen.push(&project.files);
            let loaded = project.load(|path| open.get(path).cloned());
            let index = Index::build(&loaded);
            for symbol in symbols::workspace_symbols(&index, &params.query) {
                let source = &loaded.sources[symbol.file];
                let Some(uri) = Uri::from_file_path(&source.path) else { continue };
                let range = Range::new(position_in(&source.text, symbol.span.start), position_in(&source.text, symbol.span.end));
                found.push(WorkspaceSymbol {
                    name: symbol.name,
                    kind: symbol_kind(symbol.kind),
                    tags: None,
                    container_name: None,
                    location: OneOf::Left(Location::new(uri, range)),
                    data: None,
                });
            }
        }
        Ok(Some(WorkspaceSymbolResponse::Nested(found)))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let Some(analysis) = self.analyze(&params.text_document.uri, Position::new(0, 0)).await else { return Ok(None) };
        let text = &analysis.loaded.sources[analysis.file].text;
        let ranges = symbols::folding(&analysis.loaded, analysis.file)
            .into_iter()
            .map(|span| FoldingRange {
                start_line: position_in(text, span.start).line,
                start_character: None,
                end_line: position_in(text, span.end).line,
                end_character: None,
                kind: None,
                collapsed_text: None,
            })
            .collect();
        Ok(Some(ranges))
    }

    async fn semantic_tokens_full(&self, params: SemanticTokensParams) -> Result<Option<SemanticTokensResult>> {
        let Some(analysis) = self.analyze(&params.text_document.uri, Position::new(0, 0)).await else { return Ok(None) };
        let text = &analysis.loaded.sources[analysis.file].text;
        let (mut line, mut start) = (0, 0);
        let mut data = Vec::new();
        for token in semantic::tokens(&analysis.index, &analysis.loaded, analysis.file) {
            let from = position_in(text, token.span.start);
            let to = position_in(text, token.span.end);
            let delta_line = from.line - line;
            let delta_start = if delta_line == 0 { from.character - start } else { from.character };
            data.push(SemanticToken {
                delta_line,
                delta_start,
                length: to.character.saturating_sub(from.character),
                token_type: token.kind as u32,
                token_modifiers_bitset: token.modifiers,
            });
            (line, start) = (from.line, from.character);
        }
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens { result_id: None, data })))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let Some(analysis) = self.analyze(&params.text_document.uri, params.range.start).await else { return Ok(None) };
        let text = &analysis.loaded.sources[analysis.file].text;
        let hints = inlay::hints(&analysis.index, &analysis.loaded, analysis.file)
            .into_iter()
            .map(|hint| (position_in(text, hint.offset), hint.label))
            .filter(|(position, _)| params.range.start <= *position && *position <= params.range.end)
            .map(|(position, label)| InlayHint {
                position,
                label: InlayHintLabel::String(label),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: None,
                padding_left: Some(false),
                padding_right: Some(false),
                data: None,
            })
            .collect();
        Ok(Some(hints))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let at = params.text_document_position;
        let Some(analysis) = self.analyze(&at.text_document.uri, at.position).await else { return Ok(None) };
        let items = completion::complete(&analysis.index, &analysis.loaded, analysis.file, analysis.offset)
            .into_iter()
            .map(|item| CompletionItem {
                label: item.label,
                kind: Some(match item.kind {
                    ItemKind::Keyword => CompletionItemKind::KEYWORD,
                    ItemKind::Term => CompletionItemKind::FUNCTION,
                    ItemKind::Type => CompletionItemKind::CLASS,
                    ItemKind::Variant => CompletionItemKind::ENUM_MEMBER,
                    ItemKind::Variable => CompletionItemKind::VARIABLE,
                    ItemKind::Const => CompletionItemKind::CONSTANT,
                }),
                detail: (!item.detail.is_empty()).then_some(item.detail),
                insert_text_format: item.snippet.as_ref().map(|_| InsertTextFormat::SNIPPET),
                insert_text: item.snippet,
                ..CompletionItem::default()
            })
            .collect();
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let at = params.text_document_position_params;
        let Some(analysis) = self.analyze(&at.text_document.uri, at.position).await else { return Ok(None) };
        let found = completion::signature(&analysis.index, &analysis.loaded, analysis.file, analysis.offset);
        Ok(found.map(|found| SignatureHelp {
            signatures: vec![SignatureInformation {
                label: found.label,
                documentation: None,
                parameters: Some(
                    found
                        .params
                        .into_iter()
                        .map(|(start, end)| ParameterInformation {
                            label: ParameterLabel::LabelOffsets([start as u32, end as u32]),
                            documentation: None,
                        })
                        .collect(),
                ),
                active_parameter: None,
            }],
            active_signature: Some(0),
            active_parameter: Some(found.active as u32),
        }))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let at = params.text_document_position;
        let Some(analysis) = self.analyze(&at.text_document.uri, at.position).await else { return Ok(None) };
        let places = navigation::references(&analysis.index, analysis.file, analysis.offset, params.context.include_declaration);
        Ok(Some(places.into_iter().filter_map(|place| analysis.location(place)).collect()))
    }

    async fn document_highlight(&self, params: DocumentHighlightParams) -> Result<Option<Vec<DocumentHighlight>>> {
        let at = params.text_document_position_params;
        let Some(analysis) = self.analyze(&at.text_document.uri, at.position).await else { return Ok(None) };
        let places = navigation::highlights(&analysis.index, analysis.file, analysis.offset);
        let highlights = places
            .into_iter()
            .map(|place| DocumentHighlight { range: analysis.range(place), kind: Some(DocumentHighlightKind::TEXT) })
            .collect();
        Ok(Some(highlights))
    }

    async fn prepare_rename(&self, params: TextDocumentPositionParams) -> Result<Option<PrepareRenameResponse>> {
        let Some(analysis) = self.analyze(&params.text_document.uri, params.position).await else { return Ok(None) };
        let place = navigation::prepare_rename(&analysis.index, &analysis.loaded, analysis.file, analysis.offset);
        Ok(place.map(|place| PrepareRenameResponse::Range(analysis.range(place))))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let at = params.text_document_position;
        let Some(analysis) = self.analyze(&at.text_document.uri, at.position).await else { return Ok(None) };
        let edits = navigation::rename(&analysis.index, &analysis.loaded, analysis.file, analysis.offset, &params.new_name)
            .map_err(Error::invalid_params)?;
        let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();
        for edit in edits {
            let Some(location) = analysis.location(edit.place) else { continue };
            changes.entry(location.uri).or_default().push(TextEdit { range: location.range, new_text: edit.new_text });
        }
        Ok(Some(WorkspaceEdit { changes: Some(changes), ..WorkspaceEdit::default() }))
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        let uri = params.text_document.uri;
        let Some(analysis) = self.analyze(&uri, Position::new(0, 0)).await else { return Ok(None) };
        let text = &analysis.loaded.sources[analysis.file].text;
        let mut lenses = Vec::new();
        for def in analysis.index.defs.iter().filter(|def| def.kind == DefKind::Term && def.file == analysis.file) {
            let count = rules::for_term(&analysis.loaded, &def.name).len();
            let start = position_in(text, def.name_span.start);
            let title = if count == 1 { "1 rule".to_owned() } else { format!("{count} rules") };
            lenses.push(CodeLens {
                range: Range::new(position_in(text, def.form.start), position_in(text, def.form.end)),
                command: Some(Command {
                    title,
                    command: "isle-lsp.rulesForTerm".to_owned(),
                    arguments: Some(vec![serde_json::json!(uri), serde_json::json!(start)]),
                }),
                data: None,
            });
        }
        Ok(Some(lenses))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;
        let mut actions = actions::quick_fixes(uri, &params.context.diagnostics);
        if let Some(analysis) = self.analyze(uri, params.range.start).await {
            let mut push = |title: String, kind: CodeActionKind, edit: TextEdit| {
                if actions.iter().any(|existing| existing.title == title) {
                    return;
                }
                actions.push(CodeAction {
                    title,
                    kind: Some(kind),
                    edit: Some(WorkspaceEdit { changes: Some(HashMap::from([(uri.clone(), vec![edit])])), ..WorkspaceEdit::default() }),
                    ..CodeAction::default()
                });
            };
            if let Some((title, place)) = actions::unused_decl(&analysis.index, &analysis.loaded, analysis.file, analysis.offset) {
                push(title, CodeActionKind::QUICKFIX, TextEdit { range: analysis.range(place), new_text: String::new() });
            }
            if let Some(fix) = lints::twin_fix(&analysis.loaded, analysis.file, analysis.offset) {
                let place = Place { file: analysis.file, span: fix.span };
                push(fix.title, CodeActionKind::REFACTOR, TextEdit { range: analysis.range(place), new_text: fix.new_text });
            }
            if let Some(generated) = filetest::generate(&analysis.loaded, analysis.file, analysis.offset)
                && let Some(target) = Uri::from_file_path(analysis.loaded.sources[analysis.file].path.with_file_name(&generated.file_name))
            {
                let create = DocumentChangeOperation::Op(ResourceOp::Create(CreateFile {
                    uri: target.clone(),
                    options: Some(CreateFileOptions { overwrite: Some(false), ignore_if_exists: Some(true) }),
                    annotation_id: None,
                }));
                let fill = DocumentChangeOperation::Edit(TextDocumentEdit {
                    text_document: OptionalVersionedTextDocumentIdentifier { uri: target, version: None },
                    edits: vec![OneOf::Left(TextEdit { range: Range::default(), new_text: generated.text })],
                });
                actions.push(CodeAction {
                    title: format!("Generate filetest {}", generated.file_name),
                    kind: Some(CodeActionKind::REFACTOR),
                    edit: Some(WorkspaceEdit {
                        document_changes: Some(DocumentChanges::Operations(vec![create, fill])),
                        ..WorkspaceEdit::default()
                    }),
                    ..CodeAction::default()
                });
            }
        }
        Ok(Some(actions.into_iter().map(CodeActionOrCommand::CodeAction).collect()))
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        if let Some(path) = doc.uri.to_file_path() {
            self.projects.write().await.insert(doc.uri.clone(), project::discover(&path, None));
        }
        self.documents.write().await.insert(doc.uri, Document::new(&doc.text));
        self.refresh_diagnostics().await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        {
            let mut documents = self.documents.write().await;
            let Some(document) = documents.get_mut(&params.text_document.uri) else { return };
            for change in params.content_changes {
                document.apply(change.range, &change.text);
            }
        }
        self.refresh_diagnostics().await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.write().await.remove(&uri);
        self.projects.write().await.remove(&uri);
        self.refresh_diagnostics().await;
    }
}
