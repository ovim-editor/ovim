//! An idle editor must not copy every open buffer's text on every tick just to
//! find out that nothing changed: ten 2 MB buffers cost 9% of a core at idle.

mod helpers;

use helpers::lsp_harness::FakeLsp;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Counts the bytes handed out by the allocator.
struct Counting;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATED.fetch_add(new_size.saturating_sub(layout.size()), Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ticking_an_idle_editor_does_not_copy_the_open_buffers() {
    const BUFFERS: usize = 4;
    const BYTES: usize = 2 * 1024 * 1024;

    let mut lsp = FakeLsp::start("current\n").await;
    // Hidden buffers the server has open, one per tab.
    let line = "x".repeat(79) + "\n";
    let big = line.repeat(BYTES / line.len());
    for index in 0..BUFFERS {
        let path = lsp.root(0).join(format!("big{index}.fk"));
        std::fs::write(&path, &big).unwrap();
        lsp.test.editor.new_tab();
        lsp.test.editor.open_file(&path).unwrap();
    }
    lsp.pump_until("every buffer to be opened on the server", |lsp| {
        lsp.events(0, "textDocument/didOpen").len() > BUFFERS
    })
    .await;
    // Back on the first tab: the big buffers are hidden from now on.
    lsp.test.keys("gt");
    lsp.settle().await;
    lsp.settle().await;

    let before = ALLOCATED.load(Ordering::Relaxed);
    for _ in 0..20 {
        lsp.tick().await;
    }
    let allocated = ALLOCATED.load(Ordering::Relaxed) - before;
    assert!(
        allocated < BYTES,
        "20 idle ticks allocated {allocated} bytes with {BUFFERS} hidden buffers of {BYTES} bytes"
    );
    lsp.stop().await;
}
