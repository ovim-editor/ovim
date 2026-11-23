use super::*;
use std::mem::size_of;
use std::sync::{Arc, Mutex};

/// This test measures the size of the Editor struct and its major components.
/// This is critical for understanding stack overhead and determining if we need
/// to box large fields or wrap the entire struct in Arc<Mutex<>>.
///
/// Educational context:
/// - Stack allocation is fast but limited (~2MB on most systems)
/// - CPU cache lines are typically 64 bytes
/// - Structs > 512 bytes should be considered for heap allocation
/// - Structs > 2KB definitely should be boxed or arc'd
///
/// In Rust, when you pass a value by ownership (not reference), it gets moved,
/// which involves copying all the bytes. For large structs, this becomes expensive.
#[test]
fn measure_editor_size() {
    println!("\n=== Editor Struct Size Analysis ===\n");

    let editor_size = size_of::<Editor>();
    println!(
        "Total Editor size: {} bytes ({:.2} KB)",
        editor_size,
        editor_size as f64 / 1024.0
    );

    // Measure major field types
    println!("\nMajor field sizes:");
    println!(
        "  Vec<Buffer>:                {} bytes",
        size_of::<Vec<Buffer>>()
    );
    println!(
        "  Option<WindowManager>:      {} bytes",
        size_of::<Option<WindowManager>>()
    );
    println!(
        "  RegisterManager:            {} bytes",
        size_of::<RegisterManager>()
    );
    println!(
        "  MarkManager:                {} bytes",
        size_of::<MarkManager>()
    );
    println!(
        "  KeyMapManager:              {} bytes",
        size_of::<KeyMapManager>()
    );
    println!(
        "  JumpList:                   {} bytes",
        size_of::<JumpList>()
    );
    println!(
        "  TagStack:                   {} bytes",
        size_of::<TagStack>()
    );
    println!(
        "  MacroManager:               {} bytes",
        size_of::<MacroManager>()
    );
    println!(
        "  Option<Picker>:             {} bytes",
        size_of::<Option<Picker>>()
    );
    println!(
        "  InputState:                 {} bytes",
        size_of::<InputState>()
    );
    println!(
        "  LspState:                   {} bytes",
        size_of::<LspState>()
    );
    println!(
        "  CompletionMenu:             {} bytes",
        size_of::<CompletionMenu>()
    );
    println!(
        "  HashMap<String, PreviewCache>: {} bytes",
        size_of::<HashMap<String, PreviewCache>>()
    );
    println!(
        "  ColorSchemeRegistry:        {} bytes",
        size_of::<crate::syntax::ColorSchemeRegistry>()
    );
    println!(
        "  EditorOptions:              {} bytes",
        size_of::<EditorOptions>()
    );
    println!(
        "  FileTree:                   {} bytes",
        size_of::<FileTree>()
    );
    println!(
        "  QuickfixList:               {} bytes",
        size_of::<QuickfixList>()
    );
    println!(
        "  LocationList:               {} bytes",
        size_of::<LocationList>()
    );
    println!(
        "  TabPageManager:             {} bytes",
        size_of::<TabPageManager>()
    );

    // Measure small scalar/enum fields for comparison
    println!("\nSmall field sizes (for reference):");
    println!("  Mode:                       {} bytes", size_of::<Mode>());
    println!("  bool:                       {} bytes", size_of::<bool>());
    println!("  usize:                      {} bytes", size_of::<usize>());
    println!(
        "  Option<usize>:              {} bytes",
        size_of::<Option<usize>>()
    );
    println!(
        "  Option<char>:               {} bytes",
        size_of::<Option<char>>()
    );
    println!(
        "  String:                     {} bytes",
        size_of::<String>()
    );
    println!(
        "  Option<(usize, usize)>:     {} bytes",
        size_of::<Option<(usize, usize)>>()
    );

    // Measure wrapping options
    println!("\nWrapping overhead:");
    println!(
        "  Arc<Mutex<Editor>>:         {} bytes (pointer-sized)",
        size_of::<Arc<Mutex<Editor>>>()
    );
    println!(
        "  Box<Editor>:                {} bytes (pointer-sized)",
        size_of::<Box<Editor>>()
    );

    // Analysis and recommendations
    println!("\n=== Analysis ===");

    const CACHE_LINE: usize = 64;
    const RECOMMENDED_MAX: usize = 512;
    const MUST_OPTIMIZE: usize = 2048;

    if editor_size <= CACHE_LINE {
        println!("Status: EXCELLENT - fits in a single cache line");
    } else if editor_size <= RECOMMENDED_MAX {
        println!("Status: GOOD - small enough for stack, no action needed");
    } else if editor_size <= MUST_OPTIMIZE {
        println!("Status: CONSIDER OPTIMIZATION - boxing large fields would help");
        println!("Recommendation: Box fields > 64 bytes to reduce struct size");
    } else {
        println!("Status: MUST OPTIMIZE - too large for efficient stack allocation");
        println!("Recommendation: Box all fields > 64 bytes OR wrap entire Editor in Arc<Mutex<>>");
    }

    println!("\n=== Stack Usage Patterns ===");
    println!("Current usage:");
    println!("  - run_headless_loop: takes &mut Editor (zero copy)");
    println!("  - UI rendering: borrows &Editor (zero copy)");
    println!("  - API handlers: borrow &mut Editor via channels (zero copy)");
    println!("\nVerdict: Editor is NEVER passed by value, only by reference.");
    println!(
        "Stack overhead = {} bytes once per thread (in main function).",
        editor_size
    );

    // Educational note about the measurement
    println!("\n=== Educational Context ===");
    println!("Why size matters:");
    println!(
        "1. Stack allocation: Creating Editor on stack uses {} bytes",
        editor_size
    );
    println!(
        "2. Move semantics: Moving Editor copies {} bytes",
        editor_size
    );
    println!("3. Async futures: Each .await point may store Editor state");
    println!(
        "4. Cache locality: Struct doesn't fit in L1 cache ({} bytes)",
        CACHE_LINE
    );
    println!("\nHowever, since Editor is always passed by &mut reference,");
    println!("the only overhead is the initial allocation in main().");
    println!("This is a one-time cost, not a per-call cost.");
}

/// Size regression test - fails if Editor grows beyond threshold
/// This prevents accidental struct bloat during development
#[test]
fn editor_size_regression() {
    // Rarely used per-feature state (diff review, debugger, the LSP request
    // slots) lives behind a Box, which took the struct from ~9.9 KB to
    // ~6.7 KB (OV-00472). The limit sits ~1 KB above that so the next
    // large field has to be boxed instead of quietly eating the headroom.
    const MAX_ACCEPTABLE_SIZE: usize = 7_800;

    let actual = size_of::<Editor>();

    // This test allows for some growth but prevents runaway bloat
    assert!(
        actual <= MAX_ACCEPTABLE_SIZE,
        "Editor struct is {} bytes, exceeds maximum of {} bytes. \n\
         Consider:\n\
         1. Boxing large fields (HashMap, Vec, etc.)\n\
         2. Using Arc<Mutex<>> for shared state\n\
         3. Moving large data to heap with Box\n\
         \n\
         Run 'cargo test measure_editor_size -- --nocapture' to see size breakdown.",
        actual,
        MAX_ACCEPTABLE_SIZE
    );
}

/// Test that Arc<Mutex<Editor>> is pointer-sized (8 or 16 bytes)
/// This verifies that wrapping in Arc doesn't add significant overhead
#[test]
fn arc_mutex_editor_is_pointer_sized() {
    let arc_size = size_of::<Arc<Mutex<Editor>>>();

    // Arc is a fat pointer (ptr + ref_count), should be 16 bytes on 64-bit
    assert!(
        arc_size <= 16,
        "Arc<Mutex<Editor>> should be pointer-sized (8-16 bytes), got {} bytes",
        arc_size
    );
}
