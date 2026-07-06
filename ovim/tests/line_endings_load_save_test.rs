use ovim::buffer::{Buffer, LineEnding};

fn temp_file_with_bytes(prefix: &str, bytes: &[u8]) -> tempfile::NamedTempFile {
    let file = tempfile::Builder::new()
        .prefix(prefix)
        .suffix(".txt")
        .tempfile()
        .unwrap();
    std::fs::write(file.path(), bytes).unwrap();
    file
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn load_bare_cr_file_normalizes_rope_and_detects_cr() {
    let file = temp_file_with_bytes("ovim_cr_load_", b"alpha\rbeta\rgamma");

    let buffer = Buffer::load_file_async(file.path()).await.unwrap();

    assert_eq!(buffer.line_ending(), LineEnding::Cr);
    assert_eq!(buffer.rope().to_string(), "alpha\nbeta\ngamma");
    assert!(!buffer.rope().to_string().contains('\r'));
    assert_eq!(buffer.line_text(1).unwrap(), "beta");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn save_bare_cr_file_preserves_cr_line_endings() {
    let file = temp_file_with_bytes("ovim_cr_save_", b"alpha\rbeta\rgamma");
    let mut buffer = Buffer::load_file_async(file.path()).await.unwrap();

    buffer.save_async().await.unwrap();

    assert_eq!(buffer.line_ending(), LineEnding::Cr);
    assert_eq!(std::fs::read(file.path()).unwrap(), b"alpha\rbeta\rgamma");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn load_mixed_file_normalizes_rope_and_detects_mixed() {
    let file = temp_file_with_bytes("ovim_mixed_load_", b"alpha\r\nbeta\rgamma\ndelta");

    let buffer = Buffer::load_file_async(file.path()).await.unwrap();

    assert_eq!(buffer.line_ending(), LineEnding::Mixed);
    assert_eq!(buffer.rope().to_string(), "alpha\nbeta\ngamma\ndelta");
    assert!(!buffer.rope().to_string().contains('\r'));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn save_mixed_file_writes_lf_only_and_updates_state() {
    let file = temp_file_with_bytes("ovim_mixed_save_", b"alpha\r\nbeta\rgamma\ndelta");
    let mut buffer = Buffer::load_file_async(file.path()).await.unwrap();

    buffer.save_async().await.unwrap();

    assert_eq!(buffer.line_ending(), LineEnding::Lf);
    assert_eq!(
        std::fs::read(file.path()).unwrap(),
        b"alpha\nbeta\ngamma\ndelta"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn reload_from_disk_normalizes_bare_cr_and_mixed_files() {
    let file = temp_file_with_bytes("ovim_reload_cr_", b"one\ntwo\n");
    let mut buffer = Buffer::load_file_async(file.path()).await.unwrap();

    std::fs::write(file.path(), b"one\rtwo\rthree").unwrap();
    buffer.reload_from_disk().unwrap();
    assert_eq!(buffer.line_ending(), LineEnding::Cr);
    assert_eq!(buffer.rope().to_string(), "one\ntwo\nthree");
    assert!(!buffer.rope().to_string().contains('\r'));

    std::fs::write(file.path(), b"one\r\ntwo\rthree\n").unwrap();
    buffer.reload_from_disk().unwrap();
    assert_eq!(buffer.line_ending(), LineEnding::Mixed);
    assert_eq!(buffer.rope().to_string(), "one\ntwo\nthree\n");
    assert!(!buffer.rope().to_string().contains('\r'));
}
