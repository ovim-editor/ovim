mod helpers;
use helpers::EditorTest;

#[test]
fn test_cursor_positions() {
    let mut test = EditorTest::new("number 123 end");

    println!("Initial: cursor at {:?}", test.cursor());
    test.keys("w");
    println!("After 1 w: cursor at {:?}", test.cursor());
    test.keys("w");
    println!("After 2 w: cursor at {:?}", test.cursor());
    test.keys("w");
    println!("After 3 w: cursor at {:?}", test.cursor());

    println!("\nBuffer: {}", test.buffer_content());
}
