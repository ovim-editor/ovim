//! Vertical placement of the popups that open around the cursor line
//! (completion menu, its documentation, parameter hints), so none of them
//! covers the line being typed or another popup.

use ratatui::layout::Rect;

/// Rows of the completion menu (borders included) and where its top edge
/// goes: below the cursor line when the whole menu fits there, else above it,
/// else on the side with more room with fewer rows. `None` when neither side
/// has room for a single item.
pub fn place_completion_menu(
    buffer: Rect,
    cursor_row: u16,
    wanted_height: u16,
) -> Option<(u16, u16)> {
    // Borders plus one item.
    const MIN_HEIGHT: u16 = 3;
    let below = buffer.bottom().saturating_sub(cursor_row + 1);
    let above = cursor_row.saturating_sub(buffer.y);
    let (side_below, room) = if below >= wanted_height {
        (true, below)
    } else if above >= wanted_height {
        (false, above)
    } else if below >= above {
        (true, below)
    } else {
        (false, above)
    };
    let height = wanted_height.min(room);
    if height < MIN_HEIGHT {
        return None;
    }
    let top = if side_below {
        cursor_row + 1
    } else {
        cursor_row - height
    };
    Some((top, height))
}

/// A `width` x `height` popup at column `x` that avoids the cursor line and
/// every rect in `occupied`. It prefers the rows above the cursor line (the
/// completion menu opens below it), then the rows below, then the space past
/// the popups already placed. `None` when nothing fits: callers drop the
/// popup rather than cover another one.
pub fn place_beside_cursor(
    buffer: Rect,
    x: u16,
    width: u16,
    height: u16,
    cursor_row: u16,
    occupied: &[Rect],
) -> Option<Rect> {
    let mut tops: Vec<Option<u16>> =
        vec![cursor_row.checked_sub(height), cursor_row.checked_add(1)];
    for rect in occupied {
        tops.push(rect.y.checked_sub(height));
        tops.push(rect.bottom().into());
    }
    tops.into_iter().flatten().find_map(|top| {
        let area = Rect::new(x, top, width, height);
        let in_buffer = top >= buffer.y && area.bottom() <= buffer.bottom();
        let covers_cursor = top <= cursor_row && cursor_row < area.bottom();
        let overlaps = occupied.iter().any(|rect| rect.intersects(area));
        (in_buffer && !covers_cursor && !overlaps).then_some(area)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUFFER: Rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 20,
    };

    #[test]
    fn the_menu_opens_below_the_cursor_when_it_fits() {
        assert_eq!(place_completion_menu(BUFFER, 3, 8), Some((4, 8)));
        assert_eq!(place_completion_menu(BUFFER, 11, 8), Some((12, 8)));
    }

    #[test]
    fn the_menu_flips_above_when_only_the_top_has_room() {
        assert_eq!(place_completion_menu(BUFFER, 15, 8), Some((7, 8)));
    }

    #[test]
    fn the_menu_never_covers_the_cursor_line() {
        // 20 rows, cursor on row 10: 9 rows below, 10 above. A 12 row menu
        // fits on neither side: it takes the roomier side, shortened.
        let (top, height) = place_completion_menu(BUFFER, 10, 12).unwrap();
        assert_eq!((top, height), (0, 10));
        assert!(top + height <= 10, "menu ends above the cursor line");
        let (top, height) = place_completion_menu(BUFFER, 8, 14).unwrap();
        assert_eq!((top, height), (9, 11));
        // A short viewport with the cursor in the middle.
        let short = Rect::new(0, 0, 80, 7);
        let (top, height) = place_completion_menu(short, 3, 12).unwrap();
        assert!(top > 3 || top + height <= 3, "{top} {height}");
    }

    #[test]
    fn the_menu_is_dropped_when_no_side_has_room_for_an_item() {
        let tiny = Rect::new(0, 0, 80, 5);
        assert_eq!(place_completion_menu(tiny, 2, 6), None);
    }

    #[test]
    fn hints_prefer_the_rows_above_the_cursor() {
        let area = place_beside_cursor(BUFFER, 5, 30, 4, 10, &[]).unwrap();
        assert_eq!(area, Rect::new(5, 6, 30, 4));
    }

    #[test]
    fn hints_drop_below_the_line_when_there_is_no_room_above() {
        let area = place_beside_cursor(BUFFER, 5, 30, 4, 1, &[]).unwrap();
        assert_eq!(area, Rect::new(5, 2, 30, 4));
    }

    #[test]
    fn hints_stay_clear_of_the_completion_menu() {
        // Cursor on row 1, no room above; the menu occupies rows 2..9.
        let menu = Rect::new(4, 2, 40, 7);
        let area = place_beside_cursor(BUFFER, 5, 30, 4, 1, &[menu]).unwrap();
        assert!(!area.intersects(menu));
        assert_eq!(area, Rect::new(5, 9, 30, 4), "stacked below the menu");
        // Menu flipped above the cursor: hints go above the menu.
        let menu = Rect::new(4, 13, 40, 7);
        let area = place_beside_cursor(BUFFER, 5, 30, 4, 19, &[menu]).unwrap();
        assert_eq!(area, Rect::new(5, 9, 30, 4));
    }

    #[test]
    fn hints_are_dropped_rather_than_cover_the_menu() {
        let menu = Rect::new(4, 2, 40, 17);
        assert_eq!(place_beside_cursor(BUFFER, 5, 30, 4, 1, &[menu]), None);
        // Side by side is fine: no overlap, no need to move.
        let beside = Rect::new(60, 2, 20, 10);
        assert!(place_beside_cursor(BUFFER, 5, 30, 4, 10, &[beside]).is_some());
    }
}
