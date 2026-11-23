//! Agent-oriented navigation queries
//!
//! Provides structured, data-returning navigation methods for AI agents:
//! - `get_outline` — document symbols (LSP with treesitter fallback)
//! - `search_symbols` — workspace symbol search (LSP only)
//! - `get_trace` — call hierarchy at cursor (LSP only)

use super::super::Editor;
use crate::lsp::uri_to_file_path;
use crate::navigation_types::{
    OutlineInfo, OutlineSymbol, SymbolSearchInfo, SymbolSearchResult, TraceInfo, TraceNode,
};

impl Editor {
    /// Returns a structural outline of the current document.
    ///
    /// Primary: LSP `textDocument/documentSymbol`
    /// Fallback: treesitter AST walk (Rust, TypeScript, Python)
    ///
    /// Waits for the server; callers on the editor loop use
    /// [`Self::begin_outline`] and let the loop go on meanwhile.
    pub async fn get_outline(&mut self) -> OutlineInfo {
        let file = self.buffer().file_path().map(|s| s.to_string());
        match self.begin_outline().await.await {
            Ok(outline) => outline,
            Err(_) => OutlineInfo {
                file,
                symbols: Vec::new(),
                source: "unsupported".to_string(),
                symbol_count: 0,
            },
        }
    }

    /// Starts [`Self::get_outline`]: the document is synced now, the server is
    /// asked in a task, so the editor loop never waits for its answer.
    pub async fn begin_outline(&mut self) -> tokio::task::JoinHandle<OutlineInfo> {
        let file = self.buffer().file_path().map(|s| s.to_string());
        let ctx = self.prepare_lsp_request("outline").await.ok();
        // The fallback is cheap and reads the buffer: compute it up front.
        let symbols = self.treesitter_outline();
        tokio::spawn(async move {
            if let Some(ctx) = ctx {
                let result = ctx.lsp.document_symbols(&ctx.uri, &ctx.language_id).await;
                if let Ok(symbols) = result {
                    if !symbols.is_empty() {
                        let outline_symbols: Vec<OutlineSymbol> =
                            symbols.iter().map(convert_document_symbol).collect();
                        let count = count_symbols(&outline_symbols);
                        return OutlineInfo {
                            file,
                            symbols: outline_symbols,
                            source: "lsp".to_string(),
                            symbol_count: count,
                        };
                    }
                }
            }

            // Treesitter fallback
            let count = count_symbols(&symbols);
            let source = if symbols.is_empty() {
                "unsupported"
            } else {
                "treesitter"
            };
            OutlineInfo {
                file,
                symbols,
                source: source.to_string(),
                symbol_count: count,
            }
        })
    }

    /// Searches workspace symbols by query string.
    ///
    /// LSP `workspace/symbol` only — no treesitter fallback (cross-file search needs LSP).
    /// Caps results at 50.
    pub async fn search_symbols(&mut self, query: &str) -> SymbolSearchInfo {
        let unavailable = unavailable_symbol_search(query);
        self.begin_symbol_search(query)
            .await
            .await
            .unwrap_or(unavailable)
    }

    /// Starts [`Self::search_symbols`] without waiting for the server.
    pub async fn begin_symbol_search(
        &mut self,
        query: &str,
    ) -> tokio::task::JoinHandle<SymbolSearchInfo> {
        let ctx = self.prepare_lsp_request("symbol-search").await.ok();
        let query = query.to_string();
        tokio::spawn(async move {
            if let Some(ctx) = ctx {
                let result = ctx
                    .lsp
                    .workspace_symbols(&ctx.language_id, query.clone())
                    .await;
                if let Ok(symbols) = result {
                    let results: Vec<SymbolSearchResult> = symbols
                        .iter()
                        .take(50)
                        .filter_map(|sym| {
                            let path = uri_to_file_path(&sym.location.uri)?;
                            Some(SymbolSearchResult {
                                name: sym.name.clone(),
                                kind: symbol_kind_str(sym.kind),
                                file: path.to_string_lossy().to_string(),
                                line: sym.location.range.start.line as usize + 1,
                                container: sym.container_name.clone(),
                            })
                        })
                        .collect();
                    let result_count = results.len();
                    return SymbolSearchInfo {
                        query,
                        results,
                        result_count,
                        source: "lsp".to_string(),
                    };
                }
            }
            unavailable_symbol_search(&query)
        })
    }

    /// Returns call hierarchy (incoming + outgoing calls) for the symbol at cursor.
    ///
    /// Chains 3 LSP calls: prepareCallHierarchy → incomingCalls + outgoingCalls.
    /// Caps at 50 incoming + 50 outgoing.
    pub async fn get_trace(&mut self) -> TraceInfo {
        self.begin_trace()
            .await
            .await
            .unwrap_or_else(|_| empty_trace())
    }

    /// Starts [`Self::get_trace`] without waiting for the server.
    pub async fn begin_trace(&mut self) -> tokio::task::JoinHandle<TraceInfo> {
        let ctx = self.prepare_lsp_request("trace").await.ok();
        tokio::spawn(async move {
            let Some(ctx) = ctx else {
                return empty_trace();
            };
            trace_at(&ctx).await
        })
    }

    /// Treesitter-based outline fallback.
    /// Walks the syntax tree for known node kinds per language.
    pub(crate) fn treesitter_outline(&self) -> Vec<OutlineSymbol> {
        let buffer = self.buffer();
        let tree = match buffer.syntax_tree() {
            Some(t) => t,
            None => return vec![],
        };

        let language_id = buffer
            .file_path()
            .and_then(|path| self.language_id_for_path(path));

        let node_table: &[NodeKindMapping] = match language_id.as_deref() {
            Some("rust") => RUST_NODE_TABLE,
            Some("typescript" | "typescriptreact" | "javascript" | "javascriptreact") => {
                TS_NODE_TABLE
            }
            Some("python") => PYTHON_NODE_TABLE,
            Some("java") => JAVA_NODE_TABLE,
            Some("kotlin") => KOTLIN_NODE_TABLE,
            _ => return vec![],
        };

        let root = tree.root_node();
        let source = buffer.rope().to_string();
        collect_symbols(&root, node_table, source.as_bytes())
    }
}

// --- Treesitter node tables ---

struct NodeKindMapping {
    node_kind: &'static str,
    symbol_kind: &'static str,
    name_field: &'static str,
}

static RUST_NODE_TABLE: &[NodeKindMapping] = &[
    NodeKindMapping {
        node_kind: "function_item",
        symbol_kind: "function",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "struct_item",
        symbol_kind: "struct",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "enum_item",
        symbol_kind: "enum",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "trait_item",
        symbol_kind: "interface",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "impl_item",
        symbol_kind: "impl",
        name_field: "type",
    },
    NodeKindMapping {
        node_kind: "type_item",
        symbol_kind: "type",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "const_item",
        symbol_kind: "constant",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "static_item",
        symbol_kind: "constant",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "mod_item",
        symbol_kind: "module",
        name_field: "name",
    },
];

static TS_NODE_TABLE: &[NodeKindMapping] = &[
    NodeKindMapping {
        node_kind: "function_declaration",
        symbol_kind: "function",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "class_declaration",
        symbol_kind: "class",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "interface_declaration",
        symbol_kind: "interface",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "enum_declaration",
        symbol_kind: "enum",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "type_alias_declaration",
        symbol_kind: "type",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "method_definition",
        symbol_kind: "method",
        name_field: "name",
    },
];

static PYTHON_NODE_TABLE: &[NodeKindMapping] = &[
    NodeKindMapping {
        node_kind: "function_definition",
        symbol_kind: "function",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "class_definition",
        symbol_kind: "class",
        name_field: "name",
    },
];

/// Java: the grammar names the declaration's identifier `name`.
static JAVA_NODE_TABLE: &[NodeKindMapping] = &[
    NodeKindMapping {
        node_kind: "class_declaration",
        symbol_kind: "class",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "interface_declaration",
        symbol_kind: "interface",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "enum_declaration",
        symbol_kind: "enum",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "record_declaration",
        symbol_kind: "class",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "annotation_type_declaration",
        symbol_kind: "interface",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "method_declaration",
        symbol_kind: "method",
        name_field: "name",
    },
    NodeKindMapping {
        node_kind: "constructor_declaration",
        symbol_kind: "constructor",
        name_field: "name",
    },
];

/// Kotlin (fwcd grammar): declarations have no name field, the identifier is
/// a child of the given kind (`kind:` prefix).
static KOTLIN_NODE_TABLE: &[NodeKindMapping] = &[
    NodeKindMapping {
        node_kind: "class_declaration",
        symbol_kind: "class",
        name_field: "kind:type_identifier",
    },
    NodeKindMapping {
        node_kind: "object_declaration",
        symbol_kind: "class",
        name_field: "kind:type_identifier",
    },
    NodeKindMapping {
        node_kind: "function_declaration",
        symbol_kind: "function",
        name_field: "kind:simple_identifier",
    },
    NodeKindMapping {
        node_kind: "secondary_constructor",
        symbol_kind: "constructor",
        name_field: "kind:constructor",
    },
];

// --- Helpers ---

/// The identifier node of a declaration per its table entry.
fn name_node<'a>(
    node: &tree_sitter::Node<'a>,
    mapping: &NodeKindMapping,
) -> Option<tree_sitter::Node<'a>> {
    match mapping.name_field.strip_prefix("kind:") {
        Some(kind) => {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find(|child| child.kind() == kind)
        }
        None => node.child_by_field_name(mapping.name_field),
    }
}

/// 0-based (line, char column) of `node`'s start; tree-sitter columns are bytes.
fn char_position(node: &tree_sitter::Node, source: &[u8]) -> (usize, usize) {
    let position = node.start_position();
    let line_start = node.start_byte().saturating_sub(position.column);
    let prefix = source.get(line_start..node.start_byte()).unwrap_or(&[]);
    (
        position.row,
        String::from_utf8_lossy(prefix).chars().count(),
    )
}

pub(crate) fn convert_document_symbol(sym: &lsp_types::DocumentSymbol) -> OutlineSymbol {
    let children = sym
        .children
        .as_ref()
        .map(|c| c.iter().map(convert_document_symbol).collect())
        .unwrap_or_default();
    OutlineSymbol {
        name: sym.name.clone(),
        kind: symbol_kind_str(sym.kind),
        detail: sym.detail.clone(),
        start_line: sym.range.start.line as usize + 1,
        end_line: sym.range.end.line as usize + 1,
        children,
        selection: Some((
            sym.selection_range.start.line as usize,
            sym.selection_range.start.character as usize,
        )),
    }
}

pub(crate) fn symbol_kind_str(kind: lsp_types::SymbolKind) -> String {
    match kind {
        lsp_types::SymbolKind::FILE => "file",
        lsp_types::SymbolKind::MODULE => "module",
        lsp_types::SymbolKind::NAMESPACE => "namespace",
        lsp_types::SymbolKind::PACKAGE => "package",
        lsp_types::SymbolKind::CLASS => "class",
        lsp_types::SymbolKind::METHOD => "method",
        lsp_types::SymbolKind::PROPERTY => "property",
        lsp_types::SymbolKind::FIELD => "field",
        lsp_types::SymbolKind::CONSTRUCTOR => "constructor",
        lsp_types::SymbolKind::ENUM => "enum",
        lsp_types::SymbolKind::INTERFACE => "interface",
        lsp_types::SymbolKind::FUNCTION => "function",
        lsp_types::SymbolKind::VARIABLE => "variable",
        lsp_types::SymbolKind::CONSTANT => "constant",
        lsp_types::SymbolKind::STRING => "string",
        lsp_types::SymbolKind::NUMBER => "number",
        lsp_types::SymbolKind::BOOLEAN => "boolean",
        lsp_types::SymbolKind::ARRAY => "array",
        lsp_types::SymbolKind::OBJECT => "object",
        lsp_types::SymbolKind::KEY => "key",
        lsp_types::SymbolKind::NULL => "null",
        lsp_types::SymbolKind::ENUM_MEMBER => "enum_member",
        lsp_types::SymbolKind::STRUCT => "struct",
        lsp_types::SymbolKind::EVENT => "event",
        lsp_types::SymbolKind::OPERATOR => "operator",
        lsp_types::SymbolKind::TYPE_PARAMETER => "type_parameter",
        _ => "unknown",
    }
    .to_string()
}

fn unavailable_symbol_search(query: &str) -> SymbolSearchInfo {
    SymbolSearchInfo {
        query: query.to_string(),
        results: vec![],
        result_count: 0,
        source: "unavailable".to_string(),
    }
}

fn empty_trace() -> TraceInfo {
    TraceInfo {
        target: None,
        incoming: vec![],
        outgoing: vec![],
    }
}

/// The call hierarchy at the request's position.
async fn trace_at(ctx: &super::super::lsp_integration::LspRequestContext) -> TraceInfo {
    let items = match ctx
        .lsp
        .prepare_call_hierarchy(ctx.uri.clone(), ctx.line, ctx.character, &ctx.language_id)
        .await
    {
        Ok(Some(items)) if !items.is_empty() => items,
        _ => return empty_trace(),
    };

    let item = items[0].clone();
    let target = Some(TraceNode {
        name: item.name.clone(),
        kind: symbol_kind_str(item.kind),
        file: uri_to_file_path(&item.uri)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        line: item.selection_range.start.line as usize + 1,
        detail: item.detail.clone(),
    });

    let incoming = match ctx.lsp.incoming_calls(item.clone(), &ctx.language_id).await {
        Ok(Some(calls)) => calls
            .iter()
            .take(50)
            .map(|call| TraceNode {
                name: call.from.name.clone(),
                kind: symbol_kind_str(call.from.kind),
                file: uri_to_file_path(&call.from.uri)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default(),
                line: call.from.selection_range.start.line as usize + 1,
                detail: call.from.detail.clone(),
            })
            .collect(),
        _ => vec![],
    };

    let outgoing = match ctx.lsp.outgoing_calls(item, &ctx.language_id).await {
        Ok(Some(calls)) => calls
            .iter()
            .take(50)
            .map(|call| TraceNode {
                name: call.to.name.clone(),
                kind: symbol_kind_str(call.to.kind),
                file: uri_to_file_path(&call.to.uri)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default(),
                line: call.to.selection_range.start.line as usize + 1,
                detail: call.to.detail.clone(),
            })
            .collect(),
        _ => vec![],
    };

    TraceInfo {
        target,
        incoming,
        outgoing,
    }
}

fn count_symbols(symbols: &[OutlineSymbol]) -> usize {
    symbols.iter().map(|s| 1 + count_symbols(&s.children)).sum()
}

/// Recursively collect symbols from a treesitter node.
fn collect_symbols(
    node: &tree_sitter::Node,
    table: &[NodeKindMapping],
    source: &[u8],
) -> Vec<OutlineSymbol> {
    let mut symbols = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if let Some(mapping) = table.iter().find(|m| m.node_kind == child.kind()) {
            let name_node = name_node(&child, mapping);
            let selection = name_node.as_ref().map(|n| char_position(n, source));
            let name = name_node
                .map(|n| n.utf8_text(source).unwrap_or("<unknown>").to_string())
                .unwrap_or_else(|| {
                    // For impl_item, try to build "impl Type" name
                    if mapping.node_kind == "impl_item" {
                        format!(
                            "impl {}",
                            child
                                .child_by_field_name("type")
                                .map(|n| n.utf8_text(source).unwrap_or("?"))
                                .unwrap_or("?")
                        )
                    } else {
                        "<unnamed>".to_string()
                    }
                });

            let children = collect_symbols(&child, table, source);

            symbols.push(OutlineSymbol {
                name,
                kind: mapping.symbol_kind.to_string(),
                detail: None,
                start_line: child.start_position().row + 1,
                end_line: child.end_position().row + 1,
                children,
                selection: selection.or(Some((child.start_position().row, 0))),
            });
        } else {
            // Recurse into non-matching nodes to find nested symbols
            let nested = collect_symbols(&child, table, source);
            symbols.extend(nested);
        }
    }

    symbols
}
