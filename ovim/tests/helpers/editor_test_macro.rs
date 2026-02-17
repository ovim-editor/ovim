#[path = "dsl.rs"]
pub mod dsl;

macro_rules! editor_test {
    (
        given { $( $given_line:literal )+ }
        keys $keys:literal
        expect $expect_mode:ident { $( $expect_line:literal )+ }
        $(,)?
    ) => {{
        let given_pairs: &[&str] = &[ $( $given_line ),+ ];
        let expect_pairs: &[&str] = &[ $( $expect_line ),+ ];
        let given_fixture =
            $crate::editor_test_macro::dsl::fixture_from_pairs(::ovim::mode::Mode::Normal, given_pairs);
        let expect_fixture =
            $crate::editor_test_macro::dsl::fixture_from_pairs(::ovim::mode::Mode::$expect_mode, expect_pairs);
        $crate::editor_test_macro::dsl::run_editor_test_case(given_fixture, $keys, expect_fixture);
    }};

    (
        given { $( $given_line:literal )+ }
        when $keys:literal
        expect $expect_mode:ident { $( $expect_line:literal )+ }
        $(,)?
    ) => {{
        editor_test! {
            given { $( $given_line )+ }
            keys $keys
            expect $expect_mode { $( $expect_line )+ }
        }
    }};

    (
        given { $( $given_line:literal )+ }
        $(
            keys $step_keys:literal
            expect $step_mode:ident { $( $step_line:literal )+ }
        )+
        $(,)?
    ) => {{
        let given_pairs: &[&str] = &[ $( $given_line ),+ ];
        let given_fixture =
            $crate::editor_test_macro::dsl::fixture_from_pairs(::ovim::mode::Mode::Normal, given_pairs);

        let steps: Vec<(&'static str, $crate::editor_test_macro::dsl::Fixture)> = vec![
            $(
                {
                    let expect_pairs: &[&str] = &[ $( $step_line ),+ ];
                    let expect_fixture = $crate::editor_test_macro::dsl::fixture_from_pairs(
                        ::ovim::mode::Mode::$step_mode,
                        expect_pairs,
                    );
                    ($step_keys, expect_fixture)
                }
            ),+
        ];

        $crate::editor_test_macro::dsl::run_editor_test_steps(given_fixture, steps);
    }};

    (
        given $given_mode:ident { $( $given_line:literal ),* $(,)? }
        keys $keys:literal
        expect $expect_mode:ident { $( $expect_line:literal ),* $(,)? }
        $(,)?
    ) => {{
        let given_pairs: &[&str] = &[ $( $given_line ),* ];
        let expect_pairs: &[&str] = &[ $( $expect_line ),* ];
        let given_fixture =
            $crate::editor_test_macro::dsl::fixture_from_pairs(::ovim::mode::Mode::$given_mode, given_pairs);
        let expect_fixture =
            $crate::editor_test_macro::dsl::fixture_from_pairs(::ovim::mode::Mode::$expect_mode, expect_pairs);
        $crate::editor_test_macro::dsl::run_editor_test_case(given_fixture, $keys, expect_fixture);
    }};

    (
        given $given_mode:ident { $( $given_line:literal ),* $(,)? }
        when $keys:literal
        expect $expect_mode:ident { $( $expect_line:literal ),* $(,)? }
        $(,)?
    ) => {{
        editor_test! {
            given $given_mode { $( $given_line ),* }
            keys $keys
            expect $expect_mode { $( $expect_line ),* }
        }
    }};

    (
        given $given_mode:ident { $( $given_line:literal ),* $(,)? }
        $(
            keys $step_keys:literal
            expect $step_mode:ident { $( $step_line:literal ),* $(,)? }
        )+
        $(,)?
    ) => {{
        let given_pairs: &[&str] = &[ $( $given_line ),* ];
        let given_fixture =
            $crate::editor_test_macro::dsl::fixture_from_pairs(::ovim::mode::Mode::$given_mode, given_pairs);

        let steps: Vec<(&'static str, $crate::editor_test_macro::dsl::Fixture)> = vec![
            $(
                {
                    let expect_pairs: &[&str] = &[ $( $step_line ),* ];
                    let expect_fixture = $crate::editor_test_macro::dsl::fixture_from_pairs(
                        ::ovim::mode::Mode::$step_mode,
                        expect_pairs,
                    );
                    ($step_keys, expect_fixture)
                }
            ),+
        ];

        $crate::editor_test_macro::dsl::run_editor_test_steps(given_fixture, steps);
    }};
}
