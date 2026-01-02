mod helpers;
use helpers::EditorTest;

fn main() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('o') // Open line below
        .type_text("new")
        .press_esc()
        .press('j') // Move down
        .press('.'); // Repeat

    println!("Buffer: {}", test.buffer_content());
    println!("Cursor: {:?}", test.cursor());
}
