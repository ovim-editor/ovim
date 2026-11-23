//! Breakpoints stay on their code when lines are inserted or deleted.
//!
//! Expectations come from the sign of `nvim --clean` on a buffer of `l1`..`l10`
//! with the sign on line 5 (a sign, like a breakpoint, follows its line: the
//! mark `'a` agrees except where noted). Verified with
//! `nvim --clean --headless -l` using `sign_place` + `sign_getplaced`.

mod helpers;

use helpers::EditorTest;

const FILE: &str = "/nonexistent/ovim-breakpoints/Main.java";

fn editor_with_breakpoint_on(line: usize) -> EditorTest {
    let content: String = (1..=10).map(|n| format!("l{n}\n")).collect();
    let mut t = EditorTest::new(&content);
    t.set_file_path(FILE.to_string());
    t.keys(&format!("{line}G"));
    t.editor.toggle_breakpoint();
    assert_eq!(t.editor.current_file_breakpoint_lines(), vec![line as u64]);
    t
}

/// Runs `keys` with the breakpoint on line 5 and returns where it ended up.
fn after(keys: &str) -> Vec<u64> {
    let mut t = editor_with_breakpoint_on(5);
    t.keys(keys);
    t.editor.current_file_breakpoint_lines()
}

#[test]
fn lines_added_above_push_the_breakpoint_down() {
    assert_eq!(after("4Go<Esc>"), vec![6]); // nvim: 6
    assert_eq!(after("ggyy4Gp"), vec![6]); // nvim: 6
    assert_eq!(after("ggyy4GP"), vec![6]);
    assert_eq!(after("5GO<Esc>"), vec![6]); // nvim: 6
    assert_eq!(after("5G0i<CR><Esc>"), vec![6]); // nvim: 6
}

#[test]
fn lines_added_below_or_inside_leave_it() {
    assert_eq!(after("5Go<Esc>"), vec![5]); // nvim: 5
    assert_eq!(after("5GAx<Esc>"), vec![5]); // nvim: 5
    assert_eq!(after("5G0x"), vec![5]); // nvim: 5
    assert_eq!(after("5G0li<CR><Esc>"), vec![5]); // nvim: 5
    assert_eq!(after("7Gdd"), vec![5]);
}

#[test]
fn lines_deleted_above_pull_it_up() {
    assert_eq!(after("2Gdd"), vec![4]); // nvim: 4
    assert_eq!(after("4Gdd"), vec![4]); // nvim: 4
}

#[test]
fn deleting_the_line_removes_the_breakpoint() {
    assert_eq!(after("5Gdd"), Vec::<u64>::new()); // nvim: gone
    assert_eq!(after("3G3dd"), Vec::<u64>::new()); // nvim: gone
    assert_eq!(after("4G3dd"), Vec::<u64>::new()); // nvim: gone
    assert_eq!(after("2G3dd"), vec![2]); // nvim: 2
}

#[test]
fn joining_lines_keeps_it_on_the_surviving_line() {
    assert_eq!(after("4GJ"), vec![4]); // nvim: 4
    assert_eq!(after("5GJ"), vec![5]); // nvim: 5
}

#[test]
fn undo_puts_it_back_with_its_line() {
    assert_eq!(after("2Gddu"), vec![5]);
    assert_eq!(after("4GOnew<Esc>u"), vec![5]);
}

#[test]
fn edits_to_another_file_leave_breakpoints_alone() {
    let mut t = editor_with_breakpoint_on(5);
    t.set_file_path("/nonexistent/ovim-breakpoints/Other.java".to_string());
    t.keys("ggOnew<Esc>");
    t.set_file_path(FILE.to_string());
    assert_eq!(t.editor.current_file_breakpoint_lines(), vec![5]);
}

#[test]
fn two_breakpoints_joined_into_one_line_become_one() {
    let mut t = editor_with_breakpoint_on(4);
    t.keys("5G");
    t.editor.toggle_breakpoint();
    assert_eq!(t.editor.current_file_breakpoint_lines(), vec![4, 5]);
    t.keys("4GJ");
    assert_eq!(t.editor.current_file_breakpoint_lines(), vec![4]);
}

#[test]
fn the_adapters_moved_line_follows_edits_too() {
    let mut t = editor_with_breakpoint_on(5);
    let file = std::path::Path::new(FILE);
    t.editor.dap_manager_mut().state.update_breakpoints(
        file,
        &[5],
        &[ovim_core::dap::types::DapBreakpoint {
            id: Some(1),
            verified: true,
            message: None,
            line: Some(7),
        }],
    );
    assert_eq!(t.editor.current_file_breakpoint_lines(), vec![7]);
    t.keys("ggO<Esc>");
    let bp = &t.editor.debug_state().breakpoints[file][0];
    assert_eq!((bp.line, bp.actual_line), (6, Some(8)));
}
