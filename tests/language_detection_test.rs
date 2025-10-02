use ovim::syntax::LanguageRegistry;

#[test]
fn test_language_detection_for_rust() {
    let lang = LanguageRegistry::detect_from_path("test.rs");
    println!("Detected language for test.rs: {:?}", lang);
    assert!(lang.is_some(), "Should detect Rust language for .rs files");
}

#[test]
fn test_language_detection_for_python() {
    let lang = LanguageRegistry::detect_from_path("test.py");
    println!("Detected language for test.py: {:?}", lang);
    assert!(lang.is_some(), "Should detect Python language for .py files");
}

#[test]
fn test_language_detection_for_javascript() {
    let lang = LanguageRegistry::detect_from_path("test.js");
    println!("Detected language for test.js: {:?}", lang);
    assert!(lang.is_some(), "Should detect JavaScript language for .js files");
}
