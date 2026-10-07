//! Requests for a document go to the server of the project root that owns it,
//! not to the first server started for the language.

mod helpers;

use helpers::lsp_harness::FakeLsp;
use lsp_types::{Position, Range};
use serde_json::{json, Value};

fn all_features() -> Value {
    json!({
        "textDocumentSync": {"openClose": true, "change": 2},
        "completionProvider": {"triggerCharacters": ["."]},
        "hoverProvider": true,
        "referencesProvider": true,
        "renameProvider": {"prepareProvider": true},
        "codeActionProvider": true,
        "documentFormattingProvider": true,
        "documentRangeFormattingProvider": true,
        "signatureHelpProvider": {"triggerCharacters": ["("]},
        "documentSymbolProvider": true,
        "documentHighlightProvider": true,
        "selectionRangeProvider": true,
        "foldingRangeProvider": true,
        "inlayHintProvider": true,
        "callHierarchyProvider": true,
        "semanticTokensProvider": {
            "legend": {"tokenTypes": ["variable"], "tokenModifiers": []},
            "full": true,
            "range": true,
        },
    })
}

fn range() -> Range {
    Range::new(Position::new(0, 0), Position::new(0, 1))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_request_goes_to_the_server_of_the_documents_root() {
    let mut lsp = FakeLsp::start_with("one\n", 2, all_features()).await;
    lsp.open_in_tab(1).await;
    let manager = lsp.manager();
    let uri = lsp.uri(1);
    let language = "fakels";
    let item = lsp_types::CallHierarchyItem {
        name: "item".into(),
        kind: lsp_types::SymbolKind::FUNCTION,
        tags: None,
        detail: None,
        uri: uri.clone(),
        range: range(),
        selection_range: range(),
        data: None,
    };

    // Individual failures (null answers) do not matter: the question is
    // which server received the request.
    let _ = manager.hover(&uri, 0, 0, language).await;
    let _ = manager
        .completion(
            &uri,
            0,
            0,
            language,
            ovim_core::lsp::CompletionTrigger::Invoked,
        )
        .await;
    let _ = manager.references(&uri, 0, 0, language, true).await;
    let _ = manager.prepare_rename(&uri, 0, 0, language).await;
    let _ = manager.rename(&uri, 0, 0, language, "new".into()).await;
    let _ = manager.signature_help(&uri, 0, 0, language).await;
    let _ = manager.format_document(&uri, language, 4, true).await;
    let _ = manager
        .format_range(&uri, language, 0, 0, 0, 1, 4, true)
        .await;
    let _ = manager.code_actions(&uri, 0, 0, language, vec![]).await;
    let _ = manager.document_symbols(&uri, language).await;
    let _ = manager.document_highlight(&uri, 0, 0, language).await;
    let _ = manager.selection_range(&uri, 0, 0, language).await;
    let _ = manager.folding_range(&uri, language).await;
    let _ = manager.inlay_hints(&uri, range(), language).await;
    let _ = manager.semantic_tokens_full(&uri, language).await;
    let _ = manager.semantic_tokens_range(&uri, range(), language).await;
    let _ = manager
        .prepare_call_hierarchy(uri.clone(), 0, 0, language)
        .await;
    let _ = manager.incoming_calls(item.clone(), language).await;
    let _ = manager.outgoing_calls(item, language).await;
    let _ = manager.get_semantic_tokens_legend(&uri, language).await;

    // Requests the editor fires while the second server is still starting
    // can only be answered by the first one; everything after the second
    // server has opened the document must reach the second server alone.
    let opened_at = lsp.events(1, "textDocument/didOpen")[0]["t"]
        .as_f64()
        .unwrap();
    let received = |root: usize| -> Vec<String> {
        std::fs::read_to_string(lsp.root(root).join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|event| event["params"]["textDocument"]["uri"] == uri.as_str())
            .filter(|event| event["t"].as_f64().unwrap() > opened_at)
            .filter_map(|event| event["method"].as_str().map(str::to_string))
            .filter(|method| !method.contains("didOpen") && !method.contains("didChange"))
            .collect()
    };
    assert_eq!(
        received(0),
        Vec::<String>::new(),
        "the first root's server must not see the second root's requests"
    );
    let second = received(1);
    for method in [
        "textDocument/hover",
        "textDocument/completion",
        "textDocument/references",
        "textDocument/prepareRename",
        "textDocument/rename",
        "textDocument/signatureHelp",
        "textDocument/formatting",
        "textDocument/rangeFormatting",
        "textDocument/codeAction",
        "textDocument/documentSymbol",
        "textDocument/documentHighlight",
        "textDocument/selectionRange",
        "textDocument/foldingRange",
        "textDocument/inlayHint",
        "textDocument/semanticTokens/full",
        "textDocument/semanticTokens/range",
        "textDocument/prepareCallHierarchy",
    ] {
        assert!(
            second.iter().any(|received| received == method),
            "{method} did not reach the second root's server: {second:?}"
        );
    }
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hover_key_in_second_root_asks_the_second_server() {
    let mut lsp = FakeLsp::start_with("one\n", 2, all_features()).await;
    lsp.open_in_tab(1).await;
    lsp.script(
        1,
        "response-textDocument_hover.json",
        json!({"result": {"contents": "second root hover"}}),
    );
    lsp.test.keys("K");
    lsp.wait_for_event(1, "textDocument/hover").await;
    assert!(lsp.events(0, "textDocument/hover").is_empty());
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workspace_symbols_ask_every_root() {
    let mut lsp = FakeLsp::start_with("one\n", 2, {
        let mut caps = all_features();
        caps["workspaceSymbolProvider"] = json!(true);
        caps
    })
    .await;
    lsp.open_in_tab(1).await;
    let _ = lsp.manager().workspace_symbols("fakels", "q".into()).await;
    assert_eq!(lsp.events(0, "workspace/symbol").len(), 1);
    assert_eq!(lsp.events(1, "workspace/symbol").len(), 1);
    lsp.stop().await;
}
