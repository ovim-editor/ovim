use super::ai_state::AiSelectionSnapshot;
use super::Editor;
use crate::ai::{AiContextPack, AiProfileConfig, AiRequest, CodeSlice, EditFormat};
use std::collections::HashSet;

const RELATED_SLICE_RADIUS: usize = 2;
const MAX_ITERATIONS_CAP: u8 = 3;
const TOKEN_ESTIMATE_CHAR_DIVISOR: usize = 4;

impl Editor {
    /// Builds an AI request using the profile's context policy and edit format.
    /// Returns request plus trace lines describing the local planning steps.
    pub(crate) fn build_ai_request_for_selection(
        &self,
        profile: &AiProfileConfig,
        prompt: String,
        selection: &AiSelectionSnapshot,
        edit_format: &EditFormat,
    ) -> (AiRequest, Vec<String>) {
        let file_path = self.buffer().file_path().map(ToString::to_string);
        let language_id = file_path
            .as_deref()
            .and_then(crate::syntax::LanguageRegistry::get_lsp_language_id)
            .map(ToString::to_string);

        let mut trace = vec![format!(
            "context: surrounding={}  symbols={}  related_slices={}  budget={}t",
            profile.context.surrounding_lines,
            profile.context.symbols,
            profile.context.related_slices,
            profile.context.budget,
        )];

        let context_pack = if profile.context.budget == 0 {
            trace.push("context disabled by profile policy".to_string());
            None
        } else {
            let mut pack = self.build_ai_context_pack(selection, &profile.context);
            if !profile.context.related_slices {
                trace.push("fast-path: selection + local window".to_string());
            } else {
                let max_iterations = MAX_ITERATIONS_CAP;
                let target_related = profile.context.symbols as usize;
                let per_iteration_target = if target_related == 0 {
                    0
                } else {
                    (target_related + max_iterations as usize - 1) / max_iterations as usize
                };
                let slice_radius = RELATED_SLICE_RADIUS
                    .saturating_mul(2)
                    .min(12);
                let mut seen_ranges = HashSet::new();
                for iteration in 0..max_iterations {
                    let remaining = target_related.saturating_sub(pack.related_slices.len());
                    if remaining == 0 {
                        break;
                    }
                    let iteration_target = remaining.min(per_iteration_target.max(1));
                    let added = self.expand_related_slices(
                        &mut pack.related_slices,
                        &file_path,
                        &language_id,
                        selection,
                        iteration_target,
                        slice_radius,
                        &mut seen_ranges,
                    );
                    trace.push(format!(
                        "iteration {}: expanded {} related slices (target {} radius {})",
                        iteration + 1,
                        added,
                        iteration_target,
                        slice_radius
                    ));
                    if added == 0 {
                        break;
                    }
                }
            }
            prune_context_pack_to_budget(
                &mut pack,
                profile.context.budget,
                &mut trace,
            );
            Some(pack)
        };

        (
            AiRequest {
                prompt,
                selected_text: selection.selected_text.clone(),
                language_id,
                file_path,
                edit_format: edit_format.clone(),
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
        slice_radius: usize,
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

            let start_line = line.saturating_sub(slice_radius);
            let end_line = line.saturating_add(slice_radius).min(max_line);
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

fn estimate_tokens(text: &str) -> usize {
    let char_count = text.chars().count();
    if char_count == 0 {
        0
    } else {
        (char_count + TOKEN_ESTIMATE_CHAR_DIVISOR - 1) / TOKEN_ESTIMATE_CHAR_DIVISOR
    }
}

fn estimate_code_slice_tokens(slice: &CodeSlice) -> usize {
    estimate_tokens(&slice.label) + estimate_tokens(&slice.content) + 6
}

fn estimate_context_pack_tokens(pack: &AiContextPack) -> usize {
    let mut total = estimate_tokens(&pack.selection);
    total += pack
        .surrounding
        .iter()
        .map(estimate_code_slice_tokens)
        .sum::<usize>();
    total += pack
        .related_slices
        .iter()
        .map(estimate_code_slice_tokens)
        .sum::<usize>();
    total += pack
        .symbol_facts
        .iter()
        .map(|symbol| estimate_tokens(&symbol.name) + estimate_tokens(&symbol.kind) + 4)
        .sum::<usize>();
    total += pack
        .diagnostics
        .iter()
        .map(|diag| estimate_tokens(&diag.message) + 4)
        .sum::<usize>();
    total
}

fn trim_last_content_line(content: &mut String) -> bool {
    if content.is_empty() {
        return false;
    }

    let trimmed = content.trim_end_matches('\n');
    if trimmed.is_empty() {
        content.clear();
        return false;
    }

    if let Some(last_newline) = trimmed.rfind('\n') {
        content.truncate(last_newline + 1);
    } else {
        content.clear();
    }
    true
}

fn prune_context_pack_to_budget(
    pack: &mut AiContextPack,
    budget_tokens: usize,
    trace: &mut Vec<String>,
) {
    let mut estimated = estimate_context_pack_tokens(pack);
    if estimated <= budget_tokens {
        trace.push(format!("context estimate {}t within budget", estimated));
        return;
    }

    let mut dropped_related = 0usize;
    let mut dropped_symbols = 0usize;
    let mut dropped_diagnostics = 0usize;

    while estimated > budget_tokens && !pack.related_slices.is_empty() {
        pack.related_slices.pop();
        dropped_related += 1;
        estimated = estimate_context_pack_tokens(pack);
    }

    while estimated > budget_tokens && !pack.symbol_facts.is_empty() {
        pack.symbol_facts.pop();
        dropped_symbols += 1;
        estimated = estimate_context_pack_tokens(pack);
    }

    while estimated > budget_tokens && !pack.diagnostics.is_empty() {
        pack.diagnostics.pop();
        dropped_diagnostics += 1;
        estimated = estimate_context_pack_tokens(pack);
    }

    if estimated > budget_tokens {
        for idx in 0..pack.surrounding.len() {
            while estimated > budget_tokens {
                let trimmed = {
                    let slice = &mut pack.surrounding[idx];
                    trim_last_content_line(&mut slice.content)
                };
                if !trimmed {
                    break;
                }
                estimated = estimate_context_pack_tokens(pack);
            }
            if estimated <= budget_tokens {
                break;
            }
        }
    }

    trace.push(format!(
        "context pruning applied: -{} related, -{} symbols, -{} diagnostics",
        dropped_related, dropped_symbols, dropped_diagnostics
    ));
    trace.push(format!(
        "context estimate after pruning: {}t/{}t",
        estimated, budget_tokens
    ));
}
