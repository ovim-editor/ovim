use super::ai_state::AiSelectionSnapshot;
use super::Editor;
use crate::ai::{
    AiContextPack, CodeSlice, ContextGatheringPolicy, DiagnosticFact, DiagnosticScope, SymbolFact,
};

impl Editor {
    /// Builds a compact AI context pack around the selected region.
    pub(crate) fn build_ai_context_pack(
        &self,
        selection: &AiSelectionSnapshot,
        context: &ContextGatheringPolicy,
    ) -> AiContextPack {
        let file_path = self.buffer().file_path().map(ToString::to_string);
        let language = file_path
            .as_deref()
            .and_then(crate::syntax::LanguageRegistry::get_lsp_language_id)
            .map(ToString::to_string);

        let line_count = self.buffer().line_count();
        if line_count == 0 {
            return AiContextPack {
                selection: selection.selected_text.clone(),
                surrounding: Vec::new(),
                symbol_facts: Vec::new(),
                diagnostics: Vec::new(),
                related_slices: Vec::new(),
            };
        }

        let start_line = selection
            .start_line
            .saturating_sub(context.surrounding_lines as usize);
        let end_line = selection
            .end_line
            .saturating_add(context.surrounding_lines as usize)
            .min(line_count.saturating_sub(1));

        let surrounding_content = collect_lines(self, start_line, end_line);
        let surrounding = vec![CodeSlice {
            label: "local_window".to_string(),
            path: file_path.clone(),
            language: language.clone(),
            start_line: start_line + 1,
            end_line: end_line + 1,
            content: surrounding_content,
        }];

        let diagnostics = self
            .lsp
            .state
            .current_file_diagnostics
            .iter()
            .filter(|diag| {
                if context.diagnostics == DiagnosticScope::File {
                    return true;
                }
                let diag_start = diag.range.start.line as usize;
                let diag_end = diag.range.end.line as usize;
                diag_end >= selection.start_line && diag_start <= selection.end_line
            })
            .map(|diag| DiagnosticFact {
                message: diag.message.clone(),
                severity: diag
                    .severity
                    .map(|severity| format!("{:?}", severity).to_lowercase()),
                line: diag.range.start.line + 1,
                start_character: diag.range.start.character,
                end_character: diag.range.end.character,
            })
            .collect();

        let mut symbol_facts = Vec::new();
        for symbol in &self.lsp.state.available_document_symbols {
            let line = symbol.range.start.line as usize;
            if line < start_line || line > end_line {
                continue;
            }
            symbol_facts.push(SymbolFact {
                name: symbol.name.clone(),
                kind: format!("{:?}", symbol.kind),
                line: symbol.range.start.line + 1,
                character: symbol.range.start.character,
                path: file_path.clone(),
            });
            if symbol_facts.len() >= context.symbols as usize {
                break;
            }
        }

        AiContextPack {
            selection: selection.selected_text.clone(),
            surrounding,
            symbol_facts,
            diagnostics,
            related_slices: Vec::new(),
        }
    }
}

fn collect_lines(editor: &Editor, start_line: usize, end_line: usize) -> String {
    let mut content = String::new();
    for line in start_line..=end_line {
        if let Some(text) = editor.buffer().line(line) {
            content.push_str(&text);
        }
    }
    content
}
