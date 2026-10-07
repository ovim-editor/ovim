//! Call- and type-hierarchy browser.
//!
//! The browser is a stack of picker "levels". Each row shows the symbol's name
//! and container (`detail`) plus its location, `<Enter>` jumps to it, `<Tab>`
//! drills into the selected row (callers of a caller, supertypes of a
//! supertype, ...) and `<S-Tab>` goes back up one level.

use super::super::lsp_slot::HierarchyResult;
use super::super::lsp_state::{
    HierarchyEntry, HierarchyExpand, HierarchyItem, HierarchyLevel, HierarchyState,
};
use super::super::picker::PickerResult;
use super::super::Editor;
use crate::lsp::uri_to_file_path;
use anyhow::Result;
use lsp_types::{Location, SymbolKind};

fn kind_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::CLASS => "class",
        SymbolKind::INTERFACE => "interface",
        SymbolKind::ENUM => "enum",
        SymbolKind::STRUCT => "struct",
        SymbolKind::METHOD => "method",
        SymbolKind::FUNCTION => "fn",
        SymbolKind::CONSTRUCTOR => "ctor",
        SymbolKind::FIELD | SymbolKind::PROPERTY => "field",
        SymbolKind::MODULE | SymbolKind::NAMESPACE | SymbolKind::PACKAGE => "module",
        _ => "symbol",
    }
}

fn item_parts(item: &HierarchyItem) -> (&str, Option<&str>, SymbolKind) {
    match item {
        HierarchyItem::Call(i) => (&i.name, i.detail.as_deref(), i.kind),
        HierarchyItem::Type(i) => (&i.name, i.detail.as_deref(), i.kind),
    }
}

fn call_entry(item: lsp_types::CallHierarchyItem, expand: HierarchyExpand) -> HierarchyEntry {
    let location = Location {
        uri: item.uri.clone(),
        range: item.selection_range,
    };
    HierarchyEntry {
        item: HierarchyItem::Call(item),
        expand,
        marker: "",
        location,
    }
}

fn type_entry(item: lsp_types::TypeHierarchyItem, expand: HierarchyExpand) -> HierarchyEntry {
    let location = Location {
        uri: item.uri.clone(),
        range: item.selection_range,
    };
    let marker = match expand {
        HierarchyExpand::Supertypes => "\u{2191} ",
        _ => "\u{2193} ",
    };
    HierarchyEntry {
        item: HierarchyItem::Type(item),
        expand,
        marker,
        location,
    }
}

impl Editor {
    /// One line of the hierarchy picker: `<marker><kind> <name>  <container>  <file>:<line>`.
    fn hierarchy_picker_item(&self, entry: &HierarchyEntry) -> Option<PickerResult> {
        let path = uri_to_file_path(&entry.location.uri)?;
        let (name, detail, kind) = item_parts(&entry.item);
        let line = entry.location.range.start.line as usize;
        let col = super::super::lsp_columns::ColumnResolver::new(self).grapheme_col(
            &path,
            line,
            entry.location.range.start.character,
        );
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let detail = detail.filter(|d| !d.is_empty());
        let display = match detail {
            Some(detail) => format!(
                "{}{} {}  {}  {}:{}",
                entry.marker,
                kind_label(kind),
                name,
                detail,
                file,
                line + 1
            ),
            None => format!(
                "{}{} {}  {}:{}",
                entry.marker,
                kind_label(kind),
                name,
                file,
                line + 1
            ),
        };
        Some(PickerResult {
            display,
            location: path.to_string_lossy().to_string(),
            line,
            col,
            match_positions: Vec::new(),
            content: None,
        })
    }

    /// Shows level `index` of the hierarchy stack in the picker.
    fn show_hierarchy_level(&mut self, index: usize) {
        let Some(level) = self
            .lsp
            .state
            .hierarchy
            .as_ref()
            .and_then(|state| state.levels.get(index))
            .cloned()
        else {
            return;
        };
        let items: Vec<PickerResult> = level
            .entries
            .iter()
            .filter_map(|entry| self.hierarchy_picker_item(entry))
            .collect();
        self.open_location_picker_keeping_hierarchy(items, &level.title);
        if let Some(picker) = self.picker_mut() {
            picker.set_selected_index(level.selected);
        }
        self.mark_picker_selection_changed();
        self.set_lsp_status(format!(
            "{} - <Enter> go, <Tab> expand, <S-Tab> back",
            level.title
        ));
    }

    /// True while a hierarchy picker is on screen (keys like `<Tab>` then
    /// drill down instead of switching picker fields).
    pub fn hierarchy_picker_active(&self) -> bool {
        self.lsp.state.hierarchy.is_some()
            && self.picker().is_some_and(|picker| {
                matches!(
                    picker.mode(),
                    crate::editor::picker::PickerMode::LspLocations
                )
            })
    }

    fn selected_hierarchy_entry(&self) -> Option<(usize, HierarchyEntry)> {
        let state = self.lsp.state.hierarchy.as_ref()?;
        let level_index = state.levels.len().checked_sub(1)?;
        let picked = self.picker()?.selected_result()?;
        let entries = &state.levels[level_index].entries;
        let position = entries.iter().position(|entry| {
            uri_to_file_path(&entry.location.uri)
                .is_some_and(|p| p.to_string_lossy() == picked.location)
                && entry.location.range.start.line as usize == picked.line
        })?;
        Some((position, entries[position].clone()))
    }

    /// `<Tab>` in the hierarchy picker: fetch what lies below the selected row.
    pub fn expand_selected_hierarchy_entry(&mut self) {
        let Some((position, entry)) = self.selected_hierarchy_entry() else {
            return;
        };
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return;
        };
        let Some(language_id) = self
            .lsp
            .state
            .hierarchy
            .as_ref()
            .map(|state| state.language_id.clone())
        else {
            return;
        };
        let selected = self.picker().map_or(position, |p| p.selected_index());
        if let Some(level) = self
            .lsp
            .state
            .hierarchy
            .as_mut()
            .and_then(|state| state.levels.last_mut())
        {
            level.selected = selected;
        }
        let (name, _, _) = item_parts(&entry.item);
        let title = match entry.expand {
            HierarchyExpand::Incoming => format!("Callers of {name}"),
            HierarchyExpand::Outgoing => format!("Callees of {name}"),
            HierarchyExpand::Supertypes => format!("Supertypes of {name}"),
            HierarchyExpand::Subtypes => format!("Subtypes of {name}"),
        };
        self.set_lsp_status(format!("Fetching {}...", title.to_lowercase()));

        let (tx, rx) = tokio::sync::oneshot::channel();
        let expand = entry.expand;
        let item = entry.item.clone();
        let task_language = language_id.clone();
        let task_title = title.clone();
        let task = tokio::spawn(async move {
            let entries = fetch_children(&lsp, item, expand, &task_language).await;
            let _ = tx.send(entries.map(|entries| HierarchyResult {
                empty_message: format!("Nothing found for: {task_title}"),
                title: task_title,
                entries,
                expanding: true,
                language_id: task_language,
            }));
        });
        self.lsp.slots.hierarchy_expand.fire(task, rx);
    }

    /// `<S-Tab>` in the hierarchy picker: back to the previous level.
    pub fn hierarchy_go_back(&mut self) {
        let Some(state) = self.lsp.state.hierarchy.as_mut() else {
            return;
        };
        if state.levels.len() <= 1 {
            self.set_lsp_status("Already at the top of the hierarchy".to_string());
            return;
        }
        state.levels.pop();
        let index = state.levels.len() - 1;
        self.show_hierarchy_level(index);
    }

    pub(in crate::editor) async fn call_hierarchy_impl(&mut self, incoming: bool) -> Result<bool> {
        let ctx = self.prepare_lsp_request("call-hierarchy").await?;
        let (label, expand) = if incoming {
            ("incoming calls", HierarchyExpand::Incoming)
        } else {
            ("outgoing calls", HierarchyExpand::Outgoing)
        };
        self.set_lsp_status(format!("Fetching {label}..."));

        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = async {
                let items = ctx
                    .lsp
                    .prepare_call_hierarchy(ctx.uri, ctx.line, ctx.character, &ctx.language_id)
                    .await?;
                let Some(root) = items.and_then(|items| items.into_iter().next()) else {
                    return Ok(Vec::new());
                };
                fetch_children(
                    &ctx.lsp,
                    HierarchyItem::Call(root),
                    expand,
                    &ctx.language_id,
                )
                .await
            }
            .await;
            let title = if incoming {
                "Incoming calls"
            } else {
                "Outgoing calls"
            };
            let _ = tx.send(result.map(|entries| HierarchyResult {
                title: title.to_string(),
                entries,
                empty_message: format!("No {label} found"),
                expanding: false,
                language_id: ctx.language_id.clone(),
            }));
        });
        self.lsp.slots.call_hierarchy.fire(task, rx);
        Ok(true)
    }

    pub(in crate::editor) async fn type_hierarchy_impl(&mut self) -> Result<bool> {
        let ctx = self.prepare_lsp_request("type-hierarchy").await?;
        self.set_lsp_status("Fetching type hierarchy...".to_string());

        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = async {
                let items = ctx
                    .lsp
                    .prepare_type_hierarchy(
                        ctx.uri.clone(),
                        ctx.line,
                        ctx.character,
                        &ctx.language_id,
                    )
                    .await?;
                let Some(root) = items.and_then(|items| items.into_iter().next()) else {
                    return Ok(Vec::new());
                };
                let mut entries = Vec::new();
                if let Some(supertypes) = ctx.lsp.supertypes(root.clone(), &ctx.language_id).await?
                {
                    entries.extend(
                        supertypes
                            .into_iter()
                            .map(|item| type_entry(item, HierarchyExpand::Supertypes)),
                    );
                }
                if let Some(subtypes) = ctx.lsp.subtypes(root, &ctx.language_id).await? {
                    entries.extend(
                        subtypes
                            .into_iter()
                            .map(|item| type_entry(item, HierarchyExpand::Subtypes)),
                    );
                }
                Ok(entries)
            }
            .await;
            let _ = tx.send(result.map(|entries| HierarchyResult {
                title: "Type hierarchy".to_string(),
                entries,
                empty_message: "No type hierarchy found".to_string(),
                expanding: false,
                language_id: ctx.language_id.clone(),
            }));
        });
        self.lsp.slots.type_hierarchy.fire(task, rx);
        Ok(true)
    }

    /// Applies finished hierarchy requests: opens a fresh browser for
    /// call/type hierarchy results, pushes a level for drill-downs.
    pub(in crate::editor) fn poll_hierarchy_slots(&mut self, timeout: std::time::Duration) -> bool {
        let mut changed = false;
        let mut results = Vec::new();
        if let Some(result) = self.lsp.slots.call_hierarchy.poll_with_timeout(timeout) {
            results.push(("Call hierarchy", result));
        }
        if let Some(result) = self.lsp.slots.type_hierarchy.poll_with_timeout(timeout) {
            results.push(("Type hierarchy", result));
        }
        if let Some(result) = self.lsp.slots.hierarchy_expand.poll_with_timeout(timeout) {
            results.push(("Hierarchy", result));
        }
        for (label, result) in results {
            match result {
                Ok(r) if r.entries.is_empty() => self.set_lsp_status(r.empty_message),
                Ok(r) => {
                    let count = r.entries.len();
                    let level = HierarchyLevel {
                        title: r.title.clone(),
                        entries: r.entries,
                        selected: 0,
                    };
                    if r.expanding {
                        if let Some(state) = self.lsp.state.hierarchy.as_mut() {
                            state.levels.push(level);
                        }
                    } else {
                        self.lsp.state.hierarchy = Some(HierarchyState {
                            language_id: r.language_id,
                            levels: vec![level],
                        });
                    }
                    let index = self
                        .lsp
                        .state
                        .hierarchy
                        .as_ref()
                        .map_or(0, |state| state.levels.len().saturating_sub(1));
                    self.show_hierarchy_level(index);
                    self.set_lsp_status(format!(
                        "{}: {count} - <Enter> go, <Tab> expand, <S-Tab> back",
                        r.title
                    ));
                    changed = true;
                }
                Err(error) => self.set_lsp_status(format!("{label} request failed: {error}")),
            }
        }
        changed
    }
}

async fn fetch_children(
    lsp: &crate::lsp::LspManager,
    item: HierarchyItem,
    expand: HierarchyExpand,
    language_id: &str,
) -> Result<Vec<HierarchyEntry>> {
    Ok(match (item, expand) {
        (HierarchyItem::Call(item), HierarchyExpand::Incoming) => lsp
            .incoming_calls(item, language_id)
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(|call| call_entry(call.from, HierarchyExpand::Incoming))
            .collect(),
        (HierarchyItem::Call(item), HierarchyExpand::Outgoing) => lsp
            .outgoing_calls(item, language_id)
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(|call| call_entry(call.to, HierarchyExpand::Outgoing))
            .collect(),
        (HierarchyItem::Type(item), HierarchyExpand::Supertypes) => lsp
            .supertypes(item, language_id)
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(|item| type_entry(item, HierarchyExpand::Supertypes))
            .collect(),
        (HierarchyItem::Type(item), HierarchyExpand::Subtypes) => lsp
            .subtypes(item, language_id)
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(|item| type_entry(item, HierarchyExpand::Subtypes))
            .collect(),
        _ => Vec::new(),
    })
}
