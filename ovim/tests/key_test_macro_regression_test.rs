#[macro_use]
#[path = "helpers/key_test_macro.rs"]
mod key_test_macro;
mod helpers;

#[test]
fn key_test_macro_repro_undo_cf() {
    key_test! {
        when r#"iHello good world<CR><CR><Esc>kkwdwGiprintln!("Hello world");<Esc>0cf(<Esc>u"#;
        then r#"Hello world\n\nprintln!("Hello world");"#;
    }
}

#[test]
fn key_test_macro_noexpandtab_insert_and_fix_case() {
    key_test! {
        when r#":set noexpandtab<CR>ikey_test! {<CR>when "abcdefghijlkmnop"<CR>THEN "bc"<CR><BS>}<Esc>2gg_f"lci"abc<Esc>j^cwthen<Esc>f"lci"bcdefghijlkmnop<Esc>"#;
        then r#"key_test! {\n\twhen "abc"\n\tthen "bcdefghijlkmnop"\n}\n"#;
    }
}

#[test]
fn key_test_macro_undo_redo_after_cw() {
    key_test! {
        when "iYou are a friendly individual<Esc>_2facwone<Esc>u<C-r>";
        then "You are one friendly individual";
    }
}

#[test]
fn key_test_macro_undo_redo_many_times_is_stable() {
    key_test! {
        when "iYou are a friendly individual<Esc>_2facwone<Esc>u<C-r>u<C-r>u<C-r>";
        then "You are one friendly individual";
    }
}
