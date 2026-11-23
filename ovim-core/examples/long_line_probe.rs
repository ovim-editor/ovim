//! Deterministic long-line baseline probe for core layout and input handling.
//!
//! This is intentionally an example instead of a benchmark test: it prints
//! timings for comparison between revisions, without asserting timing limits.
//! The fixture and operation order are deterministic, while wall-clock results
//! remain subject to the host machine.
//!
//! Run with the release profile for useful baselines:
//! `cargo run -p ovim-core --release --example long_line_probe -- --iterations 10`

use anyhow::{bail, Result};
use ovim_core::editor::{Editor, InputHandler};
use ovim_core::line_layout::source_fragments_for_display_range;
use ovim_core::search::Search;
use ovim_core::syntax::HighlightGroup;
use ovim_core::{KeyCode, KeyEvent, Modifiers};
use std::hint::black_box;
use std::time::{Duration, Instant};

const DEFAULT_SIZES: &[usize] = &[1_000, 10_000, 100_000, 1_000_000];
const DEFAULT_ITERATIONS: usize = 3;
const DEFAULT_WRAP_WIDTH: usize = 80;
const VIEWPORT_ROWS: usize = 8;
const VIEWPORT_COLUMNS: usize = 80;

#[derive(Debug)]
struct Config {
    sizes: Vec<usize>,
    iterations: usize,
    wrap_width: usize,
}

#[derive(Debug)]
struct Measurement {
    elapsed: Duration,
    events: usize,
    checksum: usize,
}

fn main() -> Result<()> {
    let config = parse_args(std::env::args().skip(1))?;
    println!("fixture,size_chars,operation,iterations,events,total_ms,ns_per_event,checksum");

    for &size in &config.sizes {
        for fixture in [Fixture::Plain, Fixture::Unicode] {
            let text = fixture.text(size);
            for (operation, measurement) in [
                (
                    "layout_wrap_map",
                    measure_layout(&text, config.iterations, config.wrap_width),
                ),
                (
                    "input_h_l",
                    measure_h_l(&text, config.iterations, config.wrap_width),
                ),
                (
                    "input_word_w_b",
                    measure_word_motions(&text, config.iterations, config.wrap_width),
                ),
                (
                    "input_typing",
                    measure_typing(&text, config.iterations, config.wrap_width),
                ),
                (
                    "layout_warm_row_fragments_deep",
                    measure_warm_row_fragments(&text, config.iterations, config.wrap_width),
                ),
                (
                    "layout_warm_nowrap_fragments_far",
                    measure_warm_nowrap_fragments(&text, config.iterations, config.wrap_width),
                ),
                (
                    "index_warm_far_coordinate",
                    measure_far_coordinate(&text, config.iterations),
                ),
            ] {
                print_measurement(fixture, size, operation, config.iterations, &measurement);
            }
        }

        // A single long word catches accidental whole-line materialization in
        // w's run scanner. A tiny next word makes the traversal observable:
        // unlike an EOF no-op, the cursor checksum records the landing column.
        // Setup (including cache warming) stays outside every timed operation.
        let unbroken = observable_unbroken_fixture(size);
        for (operation, measurement) in [
            (
                "input_word_w_unbroken",
                measure_word_forward(&unbroken, config.iterations, config.wrap_width),
            ),
            (
                "syntax_dense_cached_range",
                measure_cached_syntax_range(&unbroken, config.iterations),
            ),
            (
                "search_dense_cached_line",
                measure_cached_search(&unbroken, config.iterations),
            ),
        ] {
            print_measurement(
                Fixture::PlainUnbroken,
                size,
                operation,
                config.iterations,
                &measurement,
            );
        }
    }

    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Fixture {
    Plain,
    Unicode,
    PlainUnbroken,
}

impl Fixture {
    fn text(self, size: usize) -> String {
        let pattern = match self {
            Self::Plain => "word ",
            Self::Unicode => "世界 ",
            Self::PlainUnbroken => "x",
        };
        pattern
            .repeat(size.div_ceil(pattern.chars().count()))
            .chars()
            .take(size)
            .collect()
    }

    fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Unicode => "unicode",
            Self::PlainUnbroken => "plain_unbroken",
        }
    }
}

/// Keep the reported fixture size exact while giving `w` a visible destination
/// after it scans the long unbroken run. Tiny user-requested sizes remain valid.
fn observable_unbroken_fixture(size: usize) -> String {
    if size >= 3 {
        format!("{} z", Fixture::PlainUnbroken.text(size - 2))
    } else {
        Fixture::PlainUnbroken.text(size)
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Config> {
    let mut config = Config {
        sizes: DEFAULT_SIZES.to_vec(),
        iterations: DEFAULT_ITERATIONS,
        wrap_width: DEFAULT_WRAP_WIDTH,
    };
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        let mut value = |flag: &str| -> Result<String> {
            args.next()
                .ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))
        };
        match arg.as_str() {
            "--sizes" => config.sizes = parse_sizes(&value("--sizes")?)?,
            "--iterations" => {
                config.iterations = parse_positive("--iterations", &value("--iterations")?)?
            }
            "--width" => config.wrap_width = parse_positive("--width", &value("--width")?)?,
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            _ => bail!("unknown argument: {arg}\n\n{}", usage()),
        }
    }

    Ok(config)
}

fn parse_sizes(value: &str) -> Result<Vec<usize>> {
    let sizes = value
        .split(',')
        .map(|part| parse_positive("--sizes", part.trim()))
        .collect::<Result<Vec<_>>>()?;
    if sizes.is_empty() {
        bail!("--sizes requires at least one positive integer");
    }
    Ok(sizes)
}

fn parse_positive(flag: &str, value: &str) -> Result<usize> {
    let value = value
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("{flag} must be a positive integer, got {value:?}"))?;
    if value == 0 {
        bail!("{flag} must be a positive integer");
    }
    Ok(value)
}

fn usage() -> &'static str {
    "Usage: cargo run -p ovim-core --release --example long_line_probe -- [--sizes 1000,10000,100000,1000000] [--iterations 3] [--width 80]"
}

fn print_usage() {
    println!("{}", usage());
    println!("Print deterministic long-line baseline measurements as CSV.");
}

fn editor_with_fixture(text: &str) -> Editor {
    let mut editor = Editor::with_content(text);
    // Keep the probe hermetic: no operation may use the OS clipboard.
    editor.options.clipboard.clear();
    editor.options.wrap = true;
    editor
}

fn key(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), Modifiers::NONE)
}

fn dispatch(editor: &mut Editor, ch: char) {
    dispatch_event(editor, key(ch));
}

fn dispatch_event(editor: &mut Editor, event: KeyEvent) {
    InputHandler::handle_key_event(editor, event).expect("probe input command must succeed");
}

fn prepare_input_editor(text: &str, wrap_width: usize) -> Editor {
    let mut editor = editor_with_fixture(text);
    editor.ensure_wrap_map(wrap_width);
    editor
}

fn measure_layout(text: &str, iterations: usize, wrap_width: usize) -> Measurement {
    // Construct fixtures before starting the clock so this measures the real
    // Editor layout path, including WrapMap construction, rather than Rope setup.
    let mut editors = (0..iterations)
        .map(|_| editor_with_fixture(text))
        .collect::<Vec<_>>();
    let started = Instant::now();
    let checksum = editors.iter_mut().fold(0usize, |sum, editor| {
        editor.ensure_wrap_map(wrap_width);
        sum.wrapping_add(
            editor
                .wrap_map()
                .expect("wrap is enabled for layout fixture")
                .total_visual_lines(),
        )
    });

    Measurement {
        elapsed: started.elapsed(),
        events: iterations,
        checksum: black_box(checksum),
    }
}

fn measure_h_l(text: &str, iterations: usize, wrap_width: usize) -> Measurement {
    let mut editor = prepare_input_editor(text, wrap_width);
    // Begin at column one so both h and l perform a real cursor motion.
    dispatch(&mut editor, 'l');

    let started = Instant::now();
    for _ in 0..iterations {
        dispatch(&mut editor, 'h');
        dispatch(&mut editor, 'l');
    }

    Measurement {
        elapsed: started.elapsed(),
        events: iterations * 2,
        checksum: black_box(editor.buffer().cursor().col().0),
    }
}

fn measure_word_motions(text: &str, iterations: usize, wrap_width: usize) -> Measurement {
    let mut editor = prepare_input_editor(text, wrap_width);
    // Prime cursor-layout display checkpoints before measuring paired motions;
    // the timer below should capture steady-state w/b dispatch.
    dispatch(&mut editor, 'w');
    dispatch(&mut editor, 'b');

    let started = Instant::now();
    for _ in 0..iterations {
        dispatch(&mut editor, 'w');
        dispatch(&mut editor, 'b');
    }

    Measurement {
        elapsed: started.elapsed(),
        events: iterations * 2,
        checksum: black_box(editor.buffer().cursor().col().0),
    }
}

fn measure_word_forward(text: &str, iterations: usize, wrap_width: usize) -> Measurement {
    let mut editors = (0..iterations)
        .map(|_| prepare_input_editor(text, wrap_width))
        .collect::<Vec<_>>();

    // `w` on the sentinel fixture intentionally traverses the full long word.
    // Warm h/l first so any display-checkpoint construction is not folded into
    // that traversal baseline.
    for editor in &mut editors {
        dispatch(editor, 'l');
        dispatch(editor, 'h');
    }

    let started = Instant::now();
    let checksum = editors.iter_mut().fold(0usize, |sum, editor| {
        dispatch(editor, 'w');
        let cursor = editor.buffer().cursor();
        sum.wrapping_add(
            cursor
                .line()
                .wrapping_mul(text.len())
                .wrapping_add(cursor.col().0),
        )
    });

    Measurement {
        elapsed: started.elapsed(),
        events: iterations,
        checksum: black_box(checksum),
    }
}

fn measure_warm_row_fragments(text: &str, iterations: usize, wrap_width: usize) -> Measurement {
    let editor = prepare_input_editor(text, wrap_width);
    let layout = editor
        .wrap_map()
        .and_then(|map| map.line_layout(0))
        .expect("prepared fixture has first-line layout");
    let start = layout.row_count().saturating_sub(VIEWPORT_ROWS + 1);
    // The layout is built before timing; each event materializes only a fixed
    // deep viewport, independent of source-line length.
    let started = Instant::now();
    let mut checksum = 0usize;
    for _ in 0..iterations {
        let rows = layout.row_fragments(start..start + VIEWPORT_ROWS);
        checksum = checksum.wrapping_add(
            rows.iter()
                .map(|row| {
                    row.fragments
                        .iter()
                        .map(|fragment| fragment.text.len())
                        .sum::<usize>()
                })
                .sum::<usize>(),
        );
    }
    Measurement {
        elapsed: started.elapsed(),
        events: iterations,
        checksum: black_box(checksum),
    }
}

fn measure_warm_nowrap_fragments(text: &str, iterations: usize, wrap_width: usize) -> Measurement {
    let editor = prepare_input_editor(text, wrap_width);
    let layout = editor
        .wrap_map()
        .and_then(|map| map.line_layout(0))
        .expect("prepared fixture has first-line layout");
    let line = layout.line();
    let display_width = line.display_width(layout.tab_width());
    let start = display_width.saturating_sub(VIEWPORT_COLUMNS * 2);
    // This is the renderer's nowrap projection: it seeks to a far left edge
    // and materializes a fixed source fragment window.
    let started = Instant::now();
    let mut checksum = 0usize;
    for _ in 0..iterations {
        let fragments = source_fragments_for_display_range(
            line,
            layout.tab_width(),
            start..start + VIEWPORT_COLUMNS,
        );
        checksum = checksum.wrapping_add(
            fragments
                .iter()
                .map(|fragment| fragment.text.len())
                .sum::<usize>(),
        );
    }
    Measurement {
        elapsed: started.elapsed(),
        events: iterations,
        checksum: black_box(checksum),
    }
}

fn measure_far_coordinate(text: &str, iterations: usize) -> Measurement {
    let editor = editor_with_fixture(text);
    let index = editor.buffer().line_index(0);
    let tab_width = 4;
    let far_char = index.len_chars().saturating_sub(VIEWPORT_COLUMNS * 2);
    let far_display = index.char_to_display(far_char, tab_width);
    // The LineIndex is warm. Round-trip a coordinate well beyond the first
    // viewport to make any source-prefix rescans visible in the baseline.
    let started = Instant::now();
    let mut checksum = 0usize;
    for _ in 0..iterations {
        let char_col = index.display_to_char(black_box(far_display), tab_width);
        checksum =
            checksum.wrapping_add(char_col ^ black_box(index.char_to_display(char_col, tab_width)));
    }
    Measurement {
        elapsed: started.elapsed(),
        events: iterations,
        checksum: black_box(checksum),
    }
}

fn measure_cached_syntax_range(text: &str, iterations: usize) -> Measurement {
    let mut editor = editor_with_fixture(text);
    let spans = (0..text.len())
        .step_by(4)
        .map(|start| (start..(start + 2).min(text.len()), HighlightGroup::Keyword))
        .collect();
    editor.buffer_mut().set_semantic_highlights(vec![spans]);
    let start = text.len().saturating_sub(VIEWPORT_COLUMNS * 2);
    let range = start..(start + VIEWPORT_COLUMNS).min(text.len());
    // Build the interval index before timing; each event is a warm bounded
    // viewport query over densely populated syntax spans.
    let _ = editor.buffer().highlights_in_byte_range(0, range.clone());
    let started = Instant::now();
    let mut checksum = 0usize;
    for _ in 0..iterations {
        checksum = checksum.wrapping_add(
            editor
                .buffer()
                .highlights_in_byte_range(0, range.clone())
                .len(),
        );
    }
    Measurement {
        elapsed: started.elapsed(),
        events: iterations,
        checksum: black_box(checksum),
    }
}

fn measure_cached_search(text: &str, iterations: usize) -> Measurement {
    let editor = editor_with_fixture(text);
    let line = editor.buffer().line_index(0);
    let search = Search::new("x".to_string(), true);
    // Cache the dense whole-line result by immutable LineIndex identity before
    // timing repeated renderer-style queries.
    let _ = search.find_all_in_index(&line);
    let started = Instant::now();
    let mut checksum = 0usize;
    for _ in 0..iterations {
        checksum = checksum.wrapping_add(search.find_all_in_index(&line).len());
    }
    Measurement {
        elapsed: started.elapsed(),
        events: iterations,
        checksum: black_box(checksum),
    }
}

fn measure_typing(text: &str, iterations: usize, wrap_width: usize) -> Measurement {
    let mut editor = prepare_input_editor(text, wrap_width);
    dispatch(&mut editor, 'i');

    let started = Instant::now();
    for _ in 0..iterations {
        dispatch(&mut editor, 'x');
    }
    let elapsed = started.elapsed();
    dispatch_event(&mut editor, KeyEvent::new(KeyCode::Esc, Modifiers::NONE));

    Measurement {
        elapsed,
        events: iterations,
        checksum: black_box(editor.buffer().version()),
    }
}

fn print_measurement(
    fixture: Fixture,
    size: usize,
    operation: &str,
    iterations: usize,
    measurement: &Measurement,
) {
    let total_ns = measurement.elapsed.as_nanos();
    let ns_per_event = total_ns / measurement.events as u128;
    println!(
        "{},{},{},{},{},{:.3},{},{}",
        fixture.name(),
        size,
        operation,
        iterations,
        measurement.events,
        measurement.elapsed.as_secs_f64() * 1_000.0,
        ns_per_event,
        measurement.checksum,
    );
}
