use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams, InitializeParams,
    InitializeResult, ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
};
use tower_lsp_server::{Client, ClientSocket, LanguageServer, LspService};

use crate::document::Document;
use crate::project::{self, Loaded, Project};

pub fn service() -> (LspService<Backend>, ClientSocket) {
    LspService::build(Backend::new).finish()
}

pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Uri, Document>>>,
    projects: Arc<RwLock<HashMap<Uri, Project>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self { client, documents: Arc::default(), projects: Arc::default() }
    }

    pub async fn loaded(&self, uri: &Uri) -> Option<Loaded> {
        let projects = self.projects.read().await;
        let project = projects.get(uri)?;
        let documents = self.documents.read().await;
        let open: HashMap<PathBuf, &str> = documents
            .iter()
            .filter_map(|(uri, document)| Some((project::normalize(&uri.to_file_path()?), document.text())))
            .collect();
        Some(project.load(|path| open.get(path).map(|text| (*text).to_owned())))
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::INCREMENTAL)),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo { name: "isle-lsp".into(), version: Some(env!("CARGO_PKG_VERSION").into()) }),
            offset_encoding: None,
        })
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        if let Some(path) = doc.uri.to_file_path() {
            self.projects.write().await.insert(doc.uri.clone(), project::discover(&path, None));
        }
        self.documents.write().await.insert(doc.uri, Document::new(&doc.text));
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let mut documents = self.documents.write().await;
        let Some(document) = documents.get_mut(&params.text_document.uri) else { return };
        for change in params.content_changes {
            document.apply(change.range, &change.text);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.write().await.remove(&uri);
        self.projects.write().await.remove(&uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }
}
