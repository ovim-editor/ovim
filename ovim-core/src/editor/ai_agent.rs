use super::ai_state::AiSelectionSnapshot;
use super::Editor;
use crate::ai::{AgentMode, AiProfileConfig, AiRequest, CodeSlice, ExtractionStrategy};
use std::collections::HashSet;

const RELATED_SLICE_RADIUS: usize = 2;
const MAX_ITERATIONS_CAP: u8 = 3;

impl Editor {
    /// Builds an AI request using the profile's agent mode and context policy.
    /// Returns request plus trace lines describing the local planning steps.
    pub(crate) fn build_ai_request_for_selection(
        &self,
        profile: &AiProfileConfig,
        prompt: String,
        selection: &AiSelectionSnapshot,
        extraction: ExtractionStrategy,
    ) -> (AiRequest, Vec<String>) {
        let file_path = self.buffer().file_path().map(ToString::to_string);
        let language_id = file_path
            .as_deref()
            .and_then(crate::syntax::LanguageRegistry::get_lsp_language_id)
            .map(ToString::to_string);

        let mut trace = vec![format!(
            "agent mode={:?} tier={:?} budget={}t",
            profile.context_policy.mode,
            profile.context_policy.tier,
            profile.context_policy.context_budget_tokens
        )];

        let context_pack = if profile.context_policy.context_budget_tokens == 0 {
            trace.push("context disabled by profile policy".to_string());
            None
        } else {
            let mut pack = self.build_ai_context_pack(selection);
            match profile.context_policy.mode {
                AgentMode::FastPath => {
                    trace.push("fast-path: selection + local window".to_string());
                }
                AgentMode::Hybrid | AgentMode::ReactOnly => {
                    let max_iterations = profile
                        .context_policy
                        .max_iterations
                        .min(MAX_ITERATIONS_CAP)
                        .max(1);
                    let target_related = profile.context_policy.retrieval_k as usize;
                    let mut seen_ranges = HashSet::new();
                    for iteration in 0..max_iterations {
                        let remaining = target_related.saturating_sub(pack.related_slices.len());
                        if remaining == 0 {
                            break;
                        }
                        let added = self.expand_related_slices(
                            &mut pack.related_slices,
                            &file_path,
                            &language_id,
                            selection,
                            remaining,
                            &mut seen_ranges,
                        );
                        trace.push(format!(
                            "iteration {}: expanded {} related slices",
                            iteration + 1,
                            added
                        ));
                        if added == 0 {
                            break;
                        }
                    }
                }
            }
            Some(pack)
        };

        (
            AiRequest {
                prompt,
                selected_text: selection.selected_text.clone(),
                language_id,
                file_path,
                extraction,
                context_pack,
            },
            trace,
        )
    }

    fn expand_related_slices(
        &self,
        related_slices: &mut Vec<CodeSlice>,
        file_path: &Option<String>,
        language_id: &Option<String>,
        selection: &AiSelectionSnapshot,
        max_to_add: usize,
        seen_ranges: &mut HashSet<(usize, usize)>,
    ) -> usize {
        if max_to_add == 0 || self.buffer().line_count() == 0 {
            return 0;
        }

        let mut added = 0;
        let max_line = self.buffer().line_count().saturating_sub(1);
        for symbol in &self.lsp_state.available_document_symbols {
            if added >= max_to_add {
                break;
            }

            let line = symbol.selection_range.start.line as usize;
            if line >= selection.start_line && line <= selection.end_line {
                continue;
            }

            let start_line = line.saturating_sub(RELATED_SLICE_RADIUS);
            let end_line = line.saturating_add(RELATED_SLICE_RADIUS).min(max_line);
            if !seen_ranges.insert((start_line, end_line)) {
                continue;
            }

            let content = collect_lines(self, start_line, end_line);
            if content.trim().is_empty() {
                continue;
            }

            related_slices.push(CodeSlice {
                label: format!("symbol:{}", symbol.name),
                path: file_path.clone(),
                language: language_id.clone(),
                start_line: start_line + 1,
                end_line: end_line + 1,
                content,
            });
            added += 1;
        }

        added
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
