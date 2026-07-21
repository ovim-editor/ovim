use crate::editor::Editor;
use ovim_core::editor::{FileTreeAction, FileTreeClipboardKind};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

/// Renders the file tree explorer
pub fn render_file_tree(frame: &mut Frame, editor: &Editor, area: Rect) {
    if !editor.file_tree().is_visible() {
        return;
    }

    let tree = editor.file_tree();
    let flattened = tree.flattened();
    let selected_index = tree.selected_index();
    let scroll_offset = tree.scroll_offset();
    let action = tree.pending_action();

    // Reserve rows for the prompt, compact key hint, or expanded help.
    let has_prompt = !matches!(action, FileTreeAction::None);
    let footer_height = if has_prompt {
        1
    } else if tree.help_visible() {
        6.min(area.height.saturating_sub(2))
    } else {
        1.min(area.height.saturating_sub(2))
    };
    let tree_area = Rect::new(
        area.x,
        area.y,
        area.width,
        area.height.saturating_sub(footer_height),
    );

    // Calculate viewport height (area height minus border rows)
    let viewport_height = tree_area.height.saturating_sub(1) as usize;

    // Create list items from the visible portion of the flattened tree
    let items: Vec<ListItem> = flattened
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(viewport_height)
        .map(|(idx, node)| {
            let indent = "  ".repeat(node.depth());
            let icon = if node.is_dir() {
                if node.is_expanded() {
                    "\u{25bc} "
                } else {
                    "\u{25b6} "
                }
            } else {
                "  "
            };

            let name = node.name();
            let display = format!("{}{}{}", indent, icon, name);

            let style = if idx == selected_index {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else if name.starts_with('.') {
                Style::default().fg(Color::DarkGray)
            } else if node.is_dir() {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(display).style(style)
        })
        .collect();

    let visibility = match (tree.show_hidden(), tree.show_ignored()) {
        (false, false) => String::new(),
        (true, false) => " · hidden".to_string(),
        (false, true) => " · ignored".to_string(),
        (true, true) => " · hidden+ignored".to_string(),
    };
    let title = format!(" Files · {}{} ", tree.root_name(), visibility);
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(title)
                .title_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .style(Style::default().bg(Color::Rgb(30, 34, 42)));

    frame.render_widget(list, tree_area);

    let footer_area = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(footer_height),
        area.width,
        footer_height,
    );

    if has_prompt && footer_height > 0 {
        let prompt_line = match action {
            FileTreeAction::None => unreachable!(),
            FileTreeAction::Add { input } => Line::from(vec![
                Span::styled("new: ", Style::default().fg(Color::Yellow)),
                Span::raw(input.text()),
            ]),
            FileTreeAction::Rename { input, .. } => Line::from(vec![
                Span::styled("rename: ", Style::default().fg(Color::Yellow)),
                Span::raw(input.text()),
            ]),
            FileTreeAction::DeleteConfirm { name, .. } => Line::from(vec![
                Span::styled("delete ", Style::default().fg(Color::Red)),
                Span::styled(name, Style::default().fg(Color::White)),
                Span::styled("? (y/N)", Style::default().fg(Color::Red)),
            ]),
            FileTreeAction::Filter { input } => Line::from(vec![
                Span::styled("filter: ", Style::default().fg(Color::Yellow)),
                Span::raw(input.text()),
            ]),
        };
        let prompt = Paragraph::new(prompt_line).style(Style::default().bg(Color::Rgb(30, 34, 42)));
        frame.render_widget(prompt, footer_area);
    } else if tree.help_visible() && footer_height > 0 {
        let help = vec![
            Line::from("↵/l open  h close  j/k move"),
            Line::from("a add  R rename  d delete"),
            Line::from("y copy  X cut  p paste"),
            Line::from("f filter  F clear  H hidden"),
            Line::from("I ignored  r refresh  Tab focus"),
            Line::from("? help  q close"),
        ];
        frame.render_widget(
            Paragraph::new(help).style(Style::default().fg(Color::Gray).bg(Color::Rgb(30, 34, 42))),
            footer_area,
        );
    } else if footer_height > 0 {
        let clipboard = match (tree.clipboard_kind(), tree.clipboard_name()) {
            (Some(FileTreeClipboardKind::Copy), Some(name)) => format!("  COPY {name}"),
            (Some(FileTreeClipboardKind::Cut), Some(name)) => format!("  CUT {name}"),
            _ => String::new(),
        };
        let filter = if tree.filter().is_empty() {
            String::new()
        } else {
            format!("  FILTER {}", tree.filter())
        };
        let hint = format!(" ? help{filter}{clipboard}");
        frame.render_widget(
            Paragraph::new(hint).style(
                Style::default()
                    .fg(Color::DarkGray)
                    .bg(Color::Rgb(30, 34, 42)),
            ),
            footer_area,
        );
    }
}
