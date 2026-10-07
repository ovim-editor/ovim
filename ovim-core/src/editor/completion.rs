use super::completion_match::{fuzzy_match, MatchTier};
use lsp_types::CompletionItem;
use std::collections::HashSet;

/// Where the request behind the current items was made. Server `textEdit`
/// ranges index the document as it was then; if the user only typed
/// identifier characters at the cursor since, the ranges can be shifted
/// instead of thrown away (see `Editor::rebased_completion_edit_shift`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionAnchor {
    pub line: usize,
    /// Cursor char column when the request was made.
    pub col: usize,
    /// Full text of `line` when the request was made.
    pub line_text: String,
    /// Total line count of the buffer when the request was made.
    pub line_count: usize,
}

#[derive(Debug, Clone)]
struct VisibleEntry {
    /// Index into `all_items`.
    index: usize,
    /// Matched char positions inside the item's label (for highlighting).
    label_positions: Vec<usize>,
}

/// Represents a completion menu popup.
///
/// Items are kept in *server order* (`sortText`, falling back to the label);
/// filtering as the user types re-orders by match tier (prefix > camelHump >
/// subsequence) and keeps server order among equal tiers.
#[derive(Debug, Clone)]
pub struct CompletionMenu {
    /// Deduplicated items in server order.
    all_items: Vec<CompletionItem>,
    /// Items surviving the current filter, best first.
    visible_entries: Vec<VisibleEntry>,
    /// Currently selected index (into the visible list)
    selected_index: usize,
    /// Whether the menu session is active. The menu can be active but show
    /// nothing (the typed text matches no item); backspacing brings it back.
    visible: bool,
    /// The column where completion was triggered (for filtering)
    trigger_col: usize,
    /// The text that was being typed when completion was triggered
    trigger_prefix: String,
    /// Buffer version the items' textEdit ranges were computed against.
    /// Accepting an item after further edits must not apply those ranges
    /// verbatim (OV-00327).
    items_buffer_version: Option<usize>,
    /// The server said the list is partial: typing further must ask again.
    is_incomplete: bool,
    /// The user moved the selection (arrows / Ctrl-N/P / mouse).
    navigated: bool,
    /// Request position, for rebasing `textEdit` ranges after typing.
    anchor: Option<CompletionAnchor>,
    /// `all_items` indices already sent to `completionItem/resolve`.
    resolve_requested: HashSet<usize>,
    /// Bumped whenever a new list is shown; late resolve answers for an older
    /// list are dropped by comparing it.
    generation: u64,
    /// First row of the scrolled list window (see [`CompletionMenu::window`]).
    scroll_top: std::cell::Cell<usize>,
    /// The list is the choice list of a snippet `${1|a,b,c|}` stop, not a
    /// server answer: accepting replaces the stop's placeholder text.
    snippet_choices: bool,
}

impl CompletionMenu {
    /// Creates a new empty completion menu
    pub fn new() -> Self {
        Self {
            all_items: Vec::new(),
            visible_entries: Vec::new(),
            selected_index: 0,
            visible: false,
            trigger_col: 0,
            trigger_prefix: String::new(),
            items_buffer_version: None,
            is_incomplete: false,
            navigated: false,
            anchor: None,
            resolve_requested: HashSet::new(),
            generation: 0,
            scroll_top: std::cell::Cell::new(0),
            snippet_choices: false,
        }
    }

    /// Identifies the list currently shown (changes on every `show`).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Shows the completion menu with the given items
    pub fn show(&mut self, items: Vec<CompletionItem>, trigger_col: usize, trigger_prefix: String) {
        self.generation += 1;
        self.all_items = order_and_dedupe(items);
        self.selected_index = 0;
        self.scroll_top.set(0);
        self.visible = true;
        self.trigger_col = trigger_col;
        self.trigger_prefix = trigger_prefix;
        self.items_buffer_version = None;
        self.is_incomplete = false;
        self.navigated = false;
        self.anchor = None;
        self.snippet_choices = false;
        self.resolve_requested.clear();
        self.apply_filter();
    }

    /// Replaces the items with a fresh answer for the word being completed
    /// (the re-request of an `isIncomplete` list). An item the user picked
    /// stays selected if the new list still has it, so Enter does not insert
    /// a different one than the highlighted item they last saw.
    pub fn refresh(
        &mut self,
        items: Vec<CompletionItem>,
        trigger_col: usize,
        trigger_prefix: String,
    ) {
        let picked = if self.navigated {
            self.selected_item().map(item_key)
        } else {
            None
        };
        self.show(items, trigger_col, trigger_prefix);
        let position = picked.and_then(|key| {
            self.visible_entries
                .iter()
                .position(|entry| item_key(&self.all_items[entry.index]) == key)
        });
        if let Some(position) = position {
            self.selected_index = position;
            self.navigated = true;
        }
    }

    /// Shows the choices of a snippet stop (see [`Self::is_snippet_choices`]).
    pub fn show_snippet_choices(
        &mut self,
        choices: &[String],
        trigger_col: usize,
        buffer_version: usize,
    ) {
        let items = choices
            .iter()
            .enumerate()
            .map(|(index, choice)| CompletionItem {
                label: choice.clone(),
                kind: Some(lsp_types::CompletionItemKind::ENUM_MEMBER),
                // Keep the snippet's own order.
                sort_text: Some(format!("{index:06}")),
                ..Default::default()
            })
            .collect();
        self.show(items, trigger_col, String::new());
        self.items_buffer_version = Some(buffer_version);
        self.snippet_choices = true;
    }

    /// Whether the visible list is a snippet stop's choice list.
    pub fn is_snippet_choices(&self) -> bool {
        self.snippet_choices
    }

    /// Records the buffer version the current items' textEdit ranges were
    /// computed against (OV-00327).
    pub fn set_items_buffer_version(&mut self, version: usize) {
        self.items_buffer_version = Some(version);
    }

    /// The buffer version the current items' textEdit ranges target, if known.
    pub fn items_buffer_version(&self) -> Option<usize> {
        self.items_buffer_version
    }

    /// Marks the list as partial (`CompletionList.isIncomplete`).
    pub fn set_incomplete(&mut self, incomplete: bool) {
        self.is_incomplete = incomplete;
    }

    /// Whether typing further must re-ask the server.
    pub fn is_incomplete(&self) -> bool {
        self.is_incomplete
    }

    /// Records where the request behind the items was made.
    pub fn set_anchor(&mut self, anchor: CompletionAnchor) {
        self.anchor = Some(anchor);
    }

    pub fn anchor(&self) -> Option<&CompletionAnchor> {
        self.anchor.as_ref()
    }

    /// Hides the completion menu
    pub fn hide(&mut self) {
        self.visible = false;
        self.all_items.clear();
        self.visible_entries.clear();
        self.selected_index = 0;
        self.trigger_prefix.clear();
        self.items_buffer_version = None;
        self.is_incomplete = false;
        self.navigated = false;
        self.anchor = None;
        self.snippet_choices = false;
        self.resolve_requested.clear();
    }

    /// Returns whether the menu is currently visible
    pub fn is_visible(&self) -> bool {
        self.visible && !self.visible_entries.is_empty()
    }

    /// Whether a completion session is active (even if the current filter
    /// matches nothing, so the list is not drawn).
    pub fn has_session(&self) -> bool {
        self.visible
    }

    /// Number of items currently shown.
    pub fn len(&self) -> usize {
        self.visible_entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visible_entries.is_empty()
    }

    /// The `index`-th shown item.
    pub fn get(&self, index: usize) -> Option<&CompletionItem> {
        self.visible_entries
            .get(index)
            .map(|entry| &self.all_items[entry.index])
    }

    /// Items currently shown, best first.
    pub fn iter(&self) -> impl Iterator<Item = &CompletionItem> {
        self.visible_entries
            .iter()
            .map(|entry| &self.all_items[entry.index])
    }

    /// Matched char positions inside the label of the `index`-th shown item.
    pub fn matched_positions(&self, index: usize) -> &[usize] {
        self.visible_entries
            .get(index)
            .map_or(&[], |entry| &entry.label_positions)
    }

    /// The rows to draw when at most `rows` fit: the window scrolls only as
    /// far as needed to keep the selection visible (it does not jump).
    pub fn window(&self, rows: usize) -> std::ops::Range<usize> {
        let total = self.visible_entries.len();
        let rows = rows.max(1);
        let mut top = self.scroll_top.get().min(total.saturating_sub(rows));
        if self.selected_index < top {
            top = self.selected_index;
        } else if self.selected_index >= top + rows {
            top = self.selected_index + 1 - rows;
        }
        self.scroll_top.set(top);
        top..(top + rows).min(total)
    }

    /// Gets the currently selected index
    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    /// Gets the currently selected item, if any
    pub fn selected_item(&self) -> Option<&CompletionItem> {
        if self.is_visible() {
            self.get(self.selected_index)
        } else {
            None
        }
    }

    /// Whether the user moved the selection since the menu opened.
    pub fn navigated(&self) -> bool {
        self.navigated
    }

    /// Selects a visible completion item by index.
    pub fn select_index(&mut self, index: usize) -> bool {
        if index >= self.visible_entries.len() {
            return false;
        }
        self.selected_index = index;
        self.navigated = true;
        true
    }

    /// Moves the selection down by one item
    pub fn select_next(&mut self) {
        if !self.visible_entries.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.visible_entries.len();
            self.navigated = true;
        }
    }

    /// Moves the selection up by one item
    pub fn select_previous(&mut self) {
        if !self.visible_entries.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.visible_entries.len() - 1;
            } else {
                self.selected_index -= 1;
            }
            self.navigated = true;
        }
    }

    /// Gets the trigger column
    pub fn trigger_col(&self) -> usize {
        self.trigger_col
    }

    /// Gets the trigger prefix
    pub fn trigger_prefix(&self) -> &str {
        &self.trigger_prefix
    }

    /// Filters items based on current input
    pub fn filter(&mut self, current_prefix: &str) {
        if self.trigger_prefix != current_prefix {
            self.selected_index = 0;
            self.navigated = false;
        }
        self.trigger_prefix = current_prefix.to_string();
        self.apply_filter();
    }

    /// The `all_items` index behind the selected entry.
    pub fn selected_source_index(&self) -> Option<usize> {
        if !self.is_visible() {
            return None;
        }
        self.visible_entries
            .get(self.selected_index)
            .map(|entry| entry.index)
    }

    /// Marks the selected item as sent to `completionItem/resolve` and returns
    /// its source index + a copy, if it has not been requested yet.
    pub fn take_unresolved_selection(&mut self) -> Option<(usize, CompletionItem)> {
        let index = self.selected_source_index()?;
        if !self.resolve_requested.insert(index) {
            return None;
        }
        Some((index, self.all_items[index].clone()))
    }

    /// Merges a `completionItem/resolve` answer into the item at
    /// `source_index` (an `all_items` index). Only the lazily-computed
    /// fields are taken so a differing answer can never change what
    /// accepting the item inserts.
    pub fn apply_resolved(&mut self, source_index: usize, resolved: CompletionItem) {
        let Some(item) = self.all_items.get_mut(source_index) else {
            return;
        };
        if resolved.documentation.is_some() {
            item.documentation = resolved.documentation;
        }
        if resolved.detail.is_some() {
            item.detail = resolved.detail;
        }
        if resolved.label_details.is_some() {
            item.label_details = resolved.label_details;
        }
        if resolved.additional_text_edits.is_some() {
            item.additional_text_edits = resolved.additional_text_edits;
        }
    }

    fn apply_filter(&mut self) {
        let prefix = self.trigger_prefix.clone();
        let previously_selected = self.selected_source_index();

        let mut scored: Vec<(MatchTier, usize, Vec<usize>)> = Vec::new();
        for (index, item) in self.all_items.iter().enumerate() {
            if prefix.is_empty() {
                scored.push((MatchTier::Prefix, index, Vec::new()));
                continue;
            }
            let filter_text = item.filter_text.as_deref().unwrap_or(&item.label);
            let Some(found) = fuzzy_match(&prefix, filter_text) else {
                continue;
            };
            let positions = if item.filter_text.is_none() {
                found.positions
            } else {
                // The label is what is drawn: highlight what matches there.
                fuzzy_match(&prefix, &item.label)
                    .map(|m| m.positions)
                    .unwrap_or_default()
            };
            scored.push((found.tier, index, positions));
        }
        // Best tier first; `all_items` is already in server order, so the
        // (stable) sort keeps that order within a tier.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

        self.visible_entries = scored
            .into_iter()
            .map(|(_, index, label_positions)| VisibleEntry {
                index,
                label_positions,
            })
            .collect();

        // Keep the same item selected when a refresh (e.g. the isIncomplete
        // re-request) delivers a list that still contains it.
        self.selected_index = previously_selected
            .and_then(|source| {
                self.visible_entries
                    .iter()
                    .position(|entry| entry.index == source)
            })
            .filter(|_| self.navigated)
            .unwrap_or(0);
        if self.selected_index >= self.visible_entries.len() {
            self.selected_index = 0;
        }
    }
}

/// How a completion kind is drawn: one cell wide glyph plus a colour class the
/// frontends map to a theme colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionKindStyle {
    pub glyph: char,
    pub class: CompletionKindClass,
}

/// Colour families of completion kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKindClass {
    Function,
    Type,
    Variable,
    Constant,
    Keyword,
    Module,
    Snippet,
    Other,
}

impl CompletionKindClass {
    /// Stable lowercase name (CSS class suffix in the GUI).
    pub fn name(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Type => "type",
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::Keyword => "keyword",
            Self::Module => "module",
            Self::Snippet => "snippet",
            Self::Other => "other",
        }
    }
}

/// Glyph and colour class for a completion item kind.
pub fn completion_kind_style(kind: Option<lsp_types::CompletionItemKind>) -> CompletionKindStyle {
    use lsp_types::CompletionItemKind as K;
    use CompletionKindClass as C;
    let (glyph, class) = match kind {
        Some(K::METHOD) => ('m', C::Function),
        Some(K::FUNCTION) => ('f', C::Function),
        Some(K::CONSTRUCTOR) => ('+', C::Function),
        Some(K::FIELD) => ('F', C::Variable),
        Some(K::VARIABLE) => ('v', C::Variable),
        Some(K::PROPERTY) => ('p', C::Variable),
        Some(K::CLASS) => ('C', C::Type),
        Some(K::INTERFACE) => ('I', C::Type),
        Some(K::STRUCT) => ('S', C::Type),
        Some(K::ENUM) => ('E', C::Type),
        Some(K::TYPE_PARAMETER) => ('T', C::Type),
        Some(K::MODULE) => ('M', C::Module),
        Some(K::FILE) => ('f', C::Module),
        Some(K::FOLDER) => ('d', C::Module),
        Some(K::REFERENCE) => ('r', C::Module),
        Some(K::KEYWORD) => ('k', C::Keyword),
        Some(K::OPERATOR) => ('o', C::Keyword),
        Some(K::CONSTANT) => ('K', C::Constant),
        Some(K::ENUM_MEMBER) => ('e', C::Constant),
        Some(K::VALUE) => ('#', C::Constant),
        Some(K::UNIT) => ('u', C::Constant),
        Some(K::EVENT) => ('!', C::Constant),
        Some(K::SNIPPET) => ('s', C::Snippet),
        Some(K::COLOR) => ('c', C::Other),
        Some(K::TEXT) => ('t', C::Other),
        _ => ('·', C::Other),
    };
    CompletionKindStyle { glyph, class }
}

/// Whether the server marked the item deprecated (`tags` or the legacy flag).
pub fn completion_item_is_deprecated(item: &CompletionItem) -> bool {
    item.deprecated == Some(true)
        || item
            .tags
            .as_ref()
            .is_some_and(|tags| tags.contains(&lsp_types::CompletionItemTag::DEPRECATED))
}

/// The pieces of one menu row: text right after the label (`labelDetails.detail`,
/// e.g. a signature) and a right-aligned description (`labelDetails.description`,
/// e.g. the package; legacy servers' `detail` when nothing better exists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRowText {
    pub label_suffix: String,
    pub description: String,
}

pub fn completion_row_text(item: &CompletionItem) -> CompletionRowText {
    let details = item.label_details.as_ref();
    let label_suffix = details
        .and_then(|d| d.detail.clone())
        .unwrap_or_default()
        .replace('\n', " ");
    let description = details
        .and_then(|d| d.description.clone())
        .or_else(|| {
            // Legacy servers put the signature / type in `detail`.
            item.detail
                .as_deref()
                .and_then(|d| d.lines().next())
                .map(str::to_string)
        })
        .unwrap_or_default();
    CompletionRowText {
        label_suffix,
        description,
    }
}

/// Markdown for the documentation side popup: the `detail` as a code block
/// followed by the item's documentation. `None` when there is nothing to show.
pub fn completion_documentation_markdown(item: &CompletionItem) -> Option<String> {
    let documentation = item.documentation.as_ref().map(|doc| match doc {
        lsp_types::Documentation::String(text) => text.clone(),
        lsp_types::Documentation::MarkupContent(content) => match content.kind {
            lsp_types::MarkupKind::PlainText => content.value.replace('\n', "  \n"),
            lsp_types::MarkupKind::Markdown => content.value.clone(),
        },
    });
    let documentation = documentation.filter(|doc| !doc.trim().is_empty());
    let detail = item
        .detail
        .as_deref()
        .map(str::trim)
        .filter(|detail| !detail.is_empty() && *detail != item.label);
    match (detail, documentation) {
        (None, None) => None,
        (Some(detail), None) => Some(format!("```\n{detail}\n```")),
        (None, Some(doc)) => Some(doc),
        (Some(detail), Some(doc)) => Some(format!("```\n{detail}\n```\n\n{doc}")),
    }
}

/// What makes two menu rows the same suggestion: label, inserted text, and the
/// label details (so overloads stay apart).
type DedupeKey = (String, Option<String>, Option<String>, Option<String>);

/// Orders by the server's `sortText` (label when absent) and drops obvious
/// duplicates, keeping the first (best ranked) occurrence.
fn order_and_dedupe(mut items: Vec<CompletionItem>) -> Vec<CompletionItem> {
    // Stable: servers that return an already meaningful order and no
    // sortText tie-break by label, per the LSP spec.
    items.sort_by(|a, b| {
        let key_a = a.sort_text.as_deref().unwrap_or(&a.label);
        let key_b = b.sort_text.as_deref().unwrap_or(&b.label);
        key_a.cmp(key_b)
    });
    let mut seen: HashSet<DedupeKey> = HashSet::new();
    items.retain(|item| seen.insert(item_key(item)));
    items
}

/// What makes an item "the same" across answers: its label, what it inserts
/// and its details. Duplicates are dropped by it, and a selection is carried
/// over a refreshed list by it.
fn item_key(item: &CompletionItem) -> DedupeKey {
    let details = item.label_details.as_ref();
    (
        item.label.clone(),
        item.insert_text.clone().or_else(|| match &item.text_edit {
            Some(lsp_types::CompletionTextEdit::Edit(edit)) => Some(edit.new_text.clone()),
            Some(lsp_types::CompletionTextEdit::InsertAndReplace(edit)) => {
                Some(edit.new_text.clone())
            }
            None => None,
        }),
        details
            .and_then(|d| d.detail.clone())
            .or_else(|| item.detail.clone()),
        details.and_then(|d| d.description.clone()),
    )
}

impl Default for CompletionMenu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::CompletionMenu;
    use lsp_types::CompletionItem;

    fn item(label: &str) -> CompletionItem {
        CompletionItem {
            label: label.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn completion_menu_filters_by_prefix_case_insensitive() {
        let mut menu = CompletionMenu::new();
        menu.show(
            vec![item("Arc"), item("AsMut"), item("AsRef"), item("Box")],
            0,
            "as".to_string(),
        );
        let labels: Vec<String> = menu.iter().map(|i| i.label.clone()).collect();
        assert_eq!(labels, vec!["AsMut".to_string(), "AsRef".to_string()]);
    }

    #[test]
    fn completion_menu_selects_only_visible_indices() {
        let mut menu = CompletionMenu::new();
        menu.show(vec![item("Arc"), item("Box")], 0, String::new());

        assert!(menu.select_index(1));
        assert_eq!(menu.selected_index(), 1);
        assert!(!menu.select_index(2));
        assert_eq!(menu.selected_index(), 1);
    }

    #[test]
    fn completion_menu_dedupes_obvious_duplicates() {
        let mut menu = CompletionMenu::new();
        menu.show(
            vec![item("Result"), item("Result"), item("Res")],
            0,
            "re".to_string(),
        );
        let labels: Vec<String> = menu.iter().map(|i| i.label.clone()).collect();
        assert_eq!(labels, vec!["Res".to_string(), "Result".to_string()]);
    }

    #[test]
    fn completion_menu_uses_filter_text_over_label() {
        let mut menu = CompletionMenu::new();
        // Tailwind-style: label is the display name, filterText is what to match
        let mut tailwind_item = item("bg-white");
        tailwind_item.filter_text = Some("bg-white".to_string());

        let mut css_item = item("whitespace");
        css_item.filter_text = Some("whitespace".to_string());

        menu.show(vec![tailwind_item, css_item], 0, "bg-wh".to_string());
        let labels: Vec<String> = menu.iter().map(|i| i.label.clone()).collect();
        assert_eq!(labels, vec!["bg-white".to_string()]);
    }

    #[test]
    fn completion_menu_falls_back_to_label_without_filter_text() {
        let mut menu = CompletionMenu::new();
        // No filterText set — should filter by label
        menu.show(
            vec![item("forEach"), item("filter"), item("map")],
            0,
            "fo".to_string(),
        );
        let labels: Vec<String> = menu.iter().map(|i| i.label.clone()).collect();
        assert_eq!(labels, vec!["forEach".to_string()]);
    }

    fn sorted(label: &str, sort_text: &str) -> CompletionItem {
        CompletionItem {
            label: label.to_string(),
            sort_text: Some(sort_text.to_string()),
            ..Default::default()
        }
    }

    fn labels(menu: &CompletionMenu) -> Vec<String> {
        menu.iter().map(|i| i.label.clone()).collect()
    }

    #[test]
    fn menu_keeps_the_servers_sort_text_order_not_alphabetical_order() {
        let mut menu = CompletionMenu::new();
        menu.show(
            vec![
                sorted("zebra", "0001"),
                sorted("apple", "0003"),
                sorted("mango", "0002"),
                item("banana"), // no sortText: the label is the key
            ],
            0,
            String::new(),
        );
        // "0001" < "0002" < "0003" < "banana"
        assert_eq!(labels(&menu), vec!["zebra", "mango", "apple", "banana"]);
    }

    #[test]
    fn typing_orders_by_match_tier_then_server_order() {
        let mut menu = CompletionMenu::new();
        menu.show(
            vec![
                sorted("xgetEmail", "01"),   // subsequence
                sorted("getEmail", "02"),    // hump
                sorted("gemstone", "03"),    // case-insensitive prefix
                sorted("gEmpty", "04"),      // exact-case prefix
                sorted("getEmployee", "05"), // hump, later on the server
            ],
            0,
            "gEm".to_string(),
        );
        assert_eq!(
            labels(&menu),
            vec!["gEmpty", "gemstone", "getEmail", "getEmployee", "xgetEmail"]
        );
    }

    #[test]
    fn an_exact_match_comes_first_whatever_the_server_thought() {
        let mut menu = CompletionMenu::new();
        menu.show(
            vec![sorted("listOf", "01"), sorted("list", "02")],
            0,
            "list".to_string(),
        );
        assert_eq!(labels(&menu), vec!["list", "listOf"]);
    }

    #[test]
    fn narrowing_and_widening_uses_the_original_list() {
        let mut menu = CompletionMenu::new();
        menu.show(
            vec![item("getName"), item("getEmail"), item("size")],
            0,
            String::new(),
        );
        menu.filter("gE");
        assert_eq!(labels(&menu), vec!["getEmail", "getName"]);
        menu.filter("gEma");
        assert_eq!(labels(&menu), vec!["getEmail"]);
        menu.filter("");
        assert_eq!(labels(&menu), vec!["getEmail", "getName", "size"]);
        assert!(menu.has_session());
    }

    #[test]
    fn a_session_survives_a_filter_that_matches_nothing() {
        let mut menu = CompletionMenu::new();
        menu.show(vec![item("alpha")], 0, "a".to_string());
        menu.filter("az");
        assert!(!menu.is_visible());
        assert!(menu.has_session());
        menu.filter("al");
        assert!(menu.is_visible());
    }

    #[test]
    fn overloads_with_different_details_are_not_collapsed() {
        let with_detail = |detail: &str| CompletionItem {
            label: "println".to_string(),
            label_details: Some(lsp_types::CompletionItemLabelDetails {
                detail: Some(detail.to_string()),
                description: None,
            }),
            ..Default::default()
        };
        let mut menu = CompletionMenu::new();
        menu.show(
            vec![
                with_detail("(String)"),
                with_detail("(int)"),
                with_detail("(int)"),
            ],
            0,
            String::new(),
        );
        assert_eq!(menu.len(), 2);
    }

    #[test]
    fn selection_resets_when_the_text_changes_and_survives_a_refresh_only_if_chosen() {
        let mut menu = CompletionMenu::new();
        menu.show(vec![item("aa"), item("ab"), item("ac")], 0, "a".to_string());
        menu.select_next();
        assert_eq!(menu.selected_item().unwrap().label, "ab");
        menu.filter("a");
        assert_eq!(menu.selected_index(), 1, "same text: keep the choice");
        menu.filter("ab");
        assert_eq!(menu.selected_index(), 0, "new text: back to the best match");
        assert!(!menu.navigated());
    }

    #[test]
    fn hide_forgets_the_session_flags() {
        let mut menu = CompletionMenu::new();
        menu.show(vec![item("a")], 3, "a".to_string());
        menu.set_incomplete(true);
        menu.select_next();
        menu.hide();
        assert!(!menu.has_session() && !menu.is_incomplete() && !menu.navigated());
    }

    #[test]
    fn resolve_only_fills_presentation_fields() {
        let mut menu = CompletionMenu::new();
        menu.show(vec![item("alpha")], 0, "a".to_string());
        let (source, _) = menu.take_unresolved_selection().unwrap();
        assert!(menu.take_unresolved_selection().is_none(), "asked once");
        menu.apply_resolved(
            source,
            CompletionItem {
                label: "CHANGED".to_string(),
                insert_text: Some("evil".to_string()),
                detail: Some("fn alpha()".to_string()),
                documentation: Some(lsp_types::Documentation::String("docs".to_string())),
                ..Default::default()
            },
        );
        let item = menu.get(0).unwrap();
        assert_eq!(item.label, "alpha");
        assert_eq!(item.insert_text, None);
        assert_eq!(item.detail.as_deref(), Some("fn alpha()"));
        assert!(item.documentation.is_some());
    }

    #[test]
    fn the_list_window_scrolls_only_as_far_as_needed() {
        let mut menu = CompletionMenu::new();
        let many: Vec<_> = (0..30)
            .map(|i| sorted(&format!("item{i:02}"), &format!("{i:02}")))
            .collect();
        menu.show(many, 0, String::new());
        assert_eq!(menu.window(10), 0..10);
        for _ in 0..12 {
            menu.select_next();
        }
        assert_eq!(menu.window(10), 3..13, "selection 12 is the last row");
        menu.select_previous();
        menu.select_previous();
        assert_eq!(
            menu.window(10),
            3..13,
            "moving up inside the window does not scroll"
        );
        for _ in 0..9 {
            menu.select_previous();
        }
        assert_eq!(menu.window(10), 1..11);
    }

    #[test]
    fn row_text_prefers_label_details_and_falls_back_to_detail() {
        let plain = CompletionItem {
            label: "size".to_string(),
            detail: Some("fn size(&self) -> usize\nsecond line".to_string()),
            ..Default::default()
        };
        assert_eq!(
            super::completion_row_text(&plain),
            super::CompletionRowText {
                label_suffix: String::new(),
                description: "fn size(&self) -> usize".to_string()
            }
        );
        let detailed = CompletionItem {
            label: "ArrayList".to_string(),
            detail: Some("java.util.ArrayList".to_string()),
            label_details: Some(lsp_types::CompletionItemLabelDetails {
                detail: Some("<>".to_string()),
                description: Some("java.util".to_string()),
            }),
            ..Default::default()
        };
        assert_eq!(
            super::completion_row_text(&detailed),
            super::CompletionRowText {
                label_suffix: "<>".to_string(),
                description: "java.util".to_string()
            }
        );
    }

    #[test]
    fn documentation_markdown_combines_detail_and_docs() {
        let full = CompletionItem {
            label: "get".to_string(),
            detail: Some("String get()".to_string()),
            documentation: Some(lsp_types::Documentation::MarkupContent(
                lsp_types::MarkupContent {
                    kind: lsp_types::MarkupKind::Markdown,
                    value: "Returns **it**.".to_string(),
                },
            )),
            ..Default::default()
        };
        assert_eq!(
            super::completion_documentation_markdown(&full).unwrap(),
            "```\nString get()\n```\n\nReturns **it**."
        );
        assert_eq!(super::completion_documentation_markdown(&item("get")), None);
    }

    #[test]
    fn kind_styles_group_by_colour_family() {
        use lsp_types::CompletionItemKind as K;
        assert_eq!(super::completion_kind_style(Some(K::METHOD)).glyph, 'm');
        assert_eq!(
            super::completion_kind_style(Some(K::CLASS)).class,
            super::CompletionKindClass::Type
        );
        assert_eq!(super::completion_kind_style(None).class.name(), "other");
    }
}
