use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::{
    CodeActionOrCommand, CodeActionParams, CodeActionProviderCapability, CodeActionResponse, Diagnostic,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    InitializeParams, InitializeResult, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, Uri,
};
use tower_lsp_server::{Client, ClientSocket, LanguageServer, LspService};

use crate::actions;
use crate::diagnostics;
use crate::document::Document;
use crate::project::{self, Project};

pub fn service() -> (LspService<Backend>, ClientSocket) {
    LspService::build(Backend::new).finish()
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
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo { name: "isle-lsp".into(), version: Some(env!("CARGO_PKG_VERSION").into()) }),
            offset_encoding: None,
        })
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
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
