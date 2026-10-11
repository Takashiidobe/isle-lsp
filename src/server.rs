use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams, InitializeParams,
    InitializeResult, ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
};
use tower_lsp_server::{Client, ClientSocket, LanguageServer, LspService};

use crate::document::Document;

pub fn service() -> (LspService<Backend>, ClientSocket) {
    LspService::build(Backend::new).finish()
}

pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Uri, Document>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self { client, documents: Arc::default() }
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
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }
}
