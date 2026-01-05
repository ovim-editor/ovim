# Hover Window Design

## Current State
- Large centered window (80% width, 70% height)
- No markdown rendering
- No cursor-relative positioning
- Must press `q` to close

## Target State (Neovim-like)
- Positioned near cursor (prefer below, fallback above)
- Auto-sized based on content
- Markdown rendering with styled text (source hidden)
- Two modes: Preview and Navigate

## Two-Mode Behavior

### Preview Mode (First `K`)
- Shows **rendered markdown** (bold styling, code highlighting)
- Markdown source is **hidden** (user sees styled text, not `**bold**`)
- **Any key/action dismisses** the window (not just `q`)
- No scrolling - just a quick peek

### Navigate Mode (`KK` - press K twice)
- Enter from Preview mode by pressing `K` again
- Shows **raw markdown text** (for copying)
- `j/k` to scroll
- `q` or `Esc` to close
- Other keys close the window

## Wireframes

### Layout 1: Below Cursor (Preferred)
```
    1  | fn calculate(value: i32) -> i32 {
    2  |     let result = value.clamp(min, max);
    3  |     result█
         ┌─────────────────────────────────────────┐
         │ Clamp an integer value to a given range │
         │                                         │
         │ **@param** value - the value to clamp   │
         │ **@param** min - minimum (inclusive)    │
         │ **@param** max - maximum (inclusive)    │
         │                                         │
         │ ```rust                                 │
         │ fn clamp(self, min: i32, max: i32)      │
         │ ```                                     │
         │                                         │
         │ **Returns** the clamped value           │
         └─────────────────────────────────────────┘
    4  | }
```

### Layout 2: Above Cursor (Fallback when no space below)
```
         ┌─────────────────────────────────────────┐
         │ Clamp an integer value to a given range │
         │                                         │
         │ **@param** value - the value to clamp   │
         │ **@param** min - minimum (inclusive)    │
         │                                         │
         │ ```rust                                 │
         │ fn clamp(self, min: i32, max: i32)      │
         │ ```                                     │
         └─────────────────────────────────────────┘
   45  |     result█
   46  | }
   47  |
────────────────────────────────────────────────────
 NORMAL | file.rs                    45:12 | 100%
```

### Layout 3: Right of cursor (when content is short)
```
    3  |     result.cla█  ┌────────────────────────┐
                         │ fn clamp(min, max)      │
                         │ Clamp value to range    │
                         └────────────────────────┘
    4  | }
```

## Positioning Algorithm

```
1. Calculate cursor screen position (line, col)
2. Measure content dimensions (width, height)
3. Determine available space:
   - space_below = buffer_area.bottom - cursor_y - 1
   - space_above = cursor_y - buffer_area.top
   - space_right = buffer_area.right - cursor_x
   - space_left = cursor_x - buffer_area.left

4. Choose position:
   IF space_below >= content_height OR space_below >= space_above:
     position = BELOW (y = cursor_y + 1)
   ELSE:
     position = ABOVE (y = cursor_y - content_height)

5. Horizontal positioning:
   - Start at cursor_x
   - If extends beyond right edge, shift left
   - Ensure at least 1 char visible on each side
```

## Markdown Rendering

### Supported Elements

| Element | Input | Rendered |
|---------|-------|----------|
| Bold | `**text**` | Bold modifier |
| Code span | `` `code` `` | Cyan/gray background |
| Code block | ` ```lang\ncode\n``` ` | Gray bg, syntax highlight |
| Heading | `# Title` | Bold + underline |
| List item | `- item` | Bullet point |
| Horizontal rule | `---` | Line separator |

### Color Scheme

```
Background:     RGB(30, 30, 46)    - Dark blue-gray
Text:           RGB(205, 214, 244) - Light gray
Border:         RGB(137, 180, 250) - Blue
Bold:           RGB(245, 194, 231) - Pink, bold
Code span bg:   RGB(49, 50, 68)    - Slightly lighter bg
Code span fg:   RGB(148, 226, 213) - Teal
Code block bg:  RGB(24, 24, 37)    - Darker bg
Heading:        RGB(245, 194, 231) - Pink, bold, underline
```

## Sizing Rules

```
MIN_WIDTH  = 30
MAX_WIDTH  = 80
MIN_HEIGHT = 3
MAX_HEIGHT = 15

content_width  = max(line lengths) + 2 (padding)
content_height = num_lines + 2 (border)

final_width  = clamp(content_width, MIN_WIDTH, MAX_WIDTH)
final_height = clamp(content_height, MIN_HEIGHT, MAX_HEIGHT)
```

## Implementation Plan

1. **Create markdown parser** (`src/ui/markdown.rs`)
   - Parse hover text into styled spans
   - Handle code blocks, bold, inline code

2. **Update render_hover_window** (`src/ui/renderer/widgets.rs`)
   - Accept cursor position parameters
   - Implement positioning algorithm
   - Use parsed markdown for styled rendering

3. **Update call site** (`src/ui/renderer/core.rs`)
   - Pass cursor position and viewport info

4. **Add hover position to LspState** (optional)
   - Store the cursor position when hover was triggered
   - So popup stays at trigger location even if cursor moves

## Scrolling Behavior

When content exceeds MAX_HEIGHT:
- Show scroll indicator in title: "1/25 lines, j/k scroll"
- j/k scroll content within fixed window
- Window position stays anchored to cursor

## Edge Cases

1. **Cursor at bottom of screen**: Show above
2. **Cursor at top of screen**: Show below
3. **Very long lines**: Wrap within MAX_WIDTH
4. **Empty hover**: Don't show window
5. **Cursor at right edge**: Shift window left
