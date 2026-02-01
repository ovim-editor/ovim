//! Data types for agent-oriented navigation queries.
//!
//! These types are returned by the editor's LSP navigation methods
//! (outline, symbol search, call hierarchy) and used by both the
//! API layer and the editor itself.

use serde::{Deserialize, Serialize};

/// Document outline information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlineInfo {
    pub file: Option<String>,
    pub symbols: Vec<OutlineSymbol>,
    pub source: String,
    pub symbol_count: usize,
}

/// A symbol in the document outline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlineSymbol {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub children: Vec<OutlineSymbol>,
}

/// Workspace symbol search results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolSearchInfo {
    pub query: String,
    pub results: Vec<SymbolSearchResult>,
    pub result_count: usize,
    pub source: String,
}

/// A single workspace symbol search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolSearchResult {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
}

/// Call hierarchy trace information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceInfo {
    pub target: Option<TraceNode>,
    pub incoming: Vec<TraceNode>,
    pub outgoing: Vec<TraceNode>,
}

/// A node in the call hierarchy trace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceNode {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
