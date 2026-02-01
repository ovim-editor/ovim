use crate::editor::Editor;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem},
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

    // Calculate viewport height (area height minus border rows)
    let viewport_height = area.height.saturating_sub(1) as usize; // -1 for right border title area

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
            } else if node.is_dir() {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(display).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(" Files ")
                .title_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .style(Style::default().bg(Color::Rgb(30, 34, 42)));

    frame.render_widget(list, area);
}
