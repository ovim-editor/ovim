use ovim::buffer::Buffer;
use ovim::unicode::CharCol;

#[test]
fn test_buffer_version_increments_on_insert() {
    let mut buffer = Buffer::new();
    let initial_version = buffer.version();
    assert_eq!(initial_version, 0, "New buffer should start at version 0");

    buffer.insert_text_at(0, CharCol::ZERO, "a");
    assert_eq!(buffer.version(), 1, "Version should increment after insert");

    buffer.insert_text_at(0, CharCol(1), "b");
    assert_eq!(buffer.version(), 2, "Version should increment again");
}

#[test]
fn test_buffer_version_increments_on_delete() {
    let mut buffer = Buffer::new();
    buffer.insert_text_at(0, CharCol::ZERO, "a");
    let version_after_insert = buffer.version();

    buffer.delete_range(0, CharCol::ZERO, 0, CharCol(1));
    assert_eq!(
        buffer.version(),
        version_after_insert + 1,
        "Version should increment after delete"
    );
}

#[test]
fn test_buffer_version_does_not_increment_on_read() {
    let mut buffer = Buffer::new();
    buffer.insert_text_at(0, CharCol::ZERO, "a");
    let version = buffer.version();

    // Read-only operations should NOT increment version
    let _ = buffer.cursor();
    let _ = buffer.rope();
    let _ = buffer.version();

    assert_eq!(
        buffer.version(),
        version,
        "Version should not change on reads"
    );
}

#[test]
fn test_buffer_version_increments_on_replace_all() {
    let mut buffer = Buffer::new();
    buffer.insert_text_at(0, CharCol::ZERO, "initial");
    let version_after_insert = buffer.version();

    buffer.replace_all("replaced");
    assert_eq!(
        buffer.version(),
        version_after_insert + 1,
        "Version should increment after replace_all"
    );
}
