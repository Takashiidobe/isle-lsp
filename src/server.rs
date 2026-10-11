use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp_server::jsonrpc::{Error, Result};
use tower_lsp_server::ls_types::{
    CodeActionOrCommand, CodeActionParams, InlayHint, InlayHintKind, InlayHintLabel, InlayHintParams, CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams,
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
use crate::hover;
use crate::index::Index;
use crate::inlay;
use crate::navigation::{self, Place};
use crate::project::{self, Loaded, Project};

pub fn service() -> (LspService<Backend>, ClientSocket) {
    LspService::build(Backend::new).finish()
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
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["(".to_owned(), " ".to_owned()]),
                    ..CompletionOptions::default()
                }),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec![" ".to_owned()]),
                    ..SignatureHelpOptions::default()
                }),
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

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let actions = actions::quick_fixes(&params.text_document.uri, &params.context.diagnostics);
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
