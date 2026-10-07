//! The ex command table: every command ovim knows, with vim's abbreviation
//! rules, bang and range policies, argument kind, buffer-kind contexts and
//! handler. Name resolution, tab completion and file-argument completion all
//! read this one table.

use super::contexts::{Contexts, Lifecycle};
use super::{
    debug, edit, files, git, launch, lsp, options, pattern, project, quickfix, session, shell,
    windows, Ex,
};
use crate::command_result::CommandResult;
use crate::command_result::{ok, ok_silent};
use crate::editor::Editor;
use crate::editor::MapMode;
use crate::git::conflict::Resolution;
use crate::launch::LaunchMode;

pub(crate) type Handler = fn(&mut Editor, &Ex) -> CommandResult;

/// How a command treats its argument text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArgKind {
    /// No argument: anything after the name is "E488: Trailing characters".
    None,
    /// Free text; `|` starts the next command.
    Text,
    /// A file name (completed as a path); `|` starts the next command.
    /// `!cmd` instead takes the rest of the line (`:r !`, `:w !`).
    File,
    /// `/pat/rep/` — a `|` in the pattern is literal, after it separates.
    Substitute,
    /// The whole rest of the line, `|` included (`:g`, `:normal`, `:!`).
    Rest,
}

/// Which lines a command applies to without a typed range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RangePolicy {
    /// A range is "E481: No range allowed".
    None,
    /// Default: the cursor line.
    Line,
    /// Default: the cursor line; line 0 is allowed (`:0r`).
    LineOrZero,
    /// Default: the whole buffer (`:g`, `:sort`, `:w !`).
    Whole,
    /// A bare range: jump there, clamped to the buffer.
    Goto,
}

pub(crate) struct ExCommand {
    /// Names in vim's `:help` notation: `q[uit]` accepts `q`, `qu`, `qui` and
    /// `quit`. The first name is the canonical one.
    pub names: &'static [&'static str],
    pub bang: bool,
    pub range: RangePolicy,
    pub args: ArgKind,
    pub contexts: Contexts,
    /// Meaning in special buffers (chat scratch, commit message, pseudocode).
    pub lifecycle: Option<Lifecycle>,
    pub handler: Handler,
}

/// A handler that calls one editor method and reports nothing, or the
/// given message.
macro_rules! call {
    ($message:literal, $($call:tt)+) => {
        |editor, _| {
            editor.$($call)+;
            ok($message)
        }
    };
    ($($call:tt)+) => {
        |editor, _| {
            editor.$($call)+;
            ok_silent()
        }
    };
}

const fn ex(names: &'static [&'static str], handler: Handler) -> ExCommand {
    ExCommand {
        names,
        bang: false,
        range: RangePolicy::None,
        args: ArgKind::None,
        contexts: Contexts::EDITABLE,
        lifecycle: None,
        handler,
    }
}

impl std::fmt::Debug for ExCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ExCommand({})", self.names.join(", "))
    }
}

impl ExCommand {
    const fn bang(mut self) -> Self {
        self.bang = true;
        self
    }
    const fn range(mut self, range: RangePolicy) -> Self {
        self.range = range;
        self
    }
    const fn args(mut self, args: ArgKind) -> Self {
        self.args = args;
        self
    }
    /// Also allowed in the pseudocode reading view.
    const fn anywhere(mut self) -> Self {
        self.contexts = Contexts::ANY;
        self
    }
    const fn lifecycle(mut self, lifecycle: Lifecycle) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    /// The canonical full name (`quit` for `q[uit]`).
    #[cfg(test)]
    pub fn name(&self) -> String {
        full_name(self.names[0])
    }
}

use ArgKind as A;
use RangePolicy as R;

pub(crate) static COMMANDS: &[ExCommand] = &[
    // ---- line ranges and text ----
    ex(&[""], edit::goto_line).range(R::Goto).anywhere(),
    ex(&["d[elete]"], edit::delete).range(R::Line).args(A::Text),
    ex(&["y[ank]"], edit::yank).range(R::Line).args(A::Text),
    ex(&["j[oin]"], edit::join)
        .bang()
        .range(R::Line)
        .args(A::Text),
    ex(&["sor[t]"], edit::sort)
        .bang()
        .range(R::Whole)
        .args(A::Text),
    ex(&["t", "co[py]"], edit::copy)
        .range(R::Line)
        .args(A::Text),
    ex(&["m[ove]"], edit::move_lines)
        .range(R::Line)
        .args(A::Text),
    ex(&["p[rint]"], edit::print).range(R::Line),
    ex(&["u[ndo]"], edit::undo),
    ex(&["red[o]"], edit::redo),
    ex(&["norm[al]"], edit::normal)
        .bang()
        .range(R::Line)
        .args(A::Rest),
    // ---- patterns ----
    ex(&["s[ubstitute]"], pattern::substitute)
        .range(R::Line)
        .args(A::Substitute),
    ex(&["g[lobal]"], pattern::global)
        .bang()
        .range(R::Whole)
        .args(A::Rest),
    ex(&["v[global]"], pattern::global)
        .range(R::Whole)
        .args(A::Rest),
    // ---- shell ----
    ex(&["!"], shell::bang).range(R::Line).args(A::Rest),
    ex(&["r[ead]"], shell::read)
        .range(R::LineOrZero)
        .args(A::File),
    ex(&["ter[minal]", "sh[ell]"], shell::terminal).args(A::Rest),
    // ---- files ----
    ex(&["w[rite]"], files::write)
        .bang()
        .range(R::Whole)
        .args(A::File)
        .lifecycle(Lifecycle::Write),
    ex(&["wq"], files::write_quit)
        .bang()
        .args(A::File)
        .lifecycle(Lifecycle::Write),
    ex(&["x[it]", "exi[t]"], files::xit)
        .bang()
        .args(A::File)
        .lifecycle(Lifecycle::Write),
    ex(&["wa[ll]", "writeall"], files::write_all).bang(),
    ex(&["wqa[ll]", "xa[ll]"], files::write_all_quit).bang(),
    ex(&["up[date]"], files::update).bang().args(A::File),
    ex(&["sav[eas]"], files::save_as).bang().args(A::File),
    ex(&["e[dit]"], files::edit).bang().args(A::File).anywhere(),
    ex(&["checkt[ime]"], files::checktime),
    ex(&["rec[over]"], files::recover).bang(),
    ex(&["f[ile]"], files::file_info),
    ex(&["pw[d]"], files::pwd),
    ex(&["cd", "lc[d]"], files::cd).args(A::File),
    // ---- quitting, windows, tab pages, buffers ----
    ex(&["q[uit]"], windows::quit)
        .bang()
        .lifecycle(Lifecycle::Quit),
    ex(&["qa[ll]", "quita[ll]"], windows::quit_all)
        .bang()
        .anywhere(),
    ex(&["cq[uit]"], windows::cquit).bang().args(A::Text),
    ex(&["clo[se]"], windows::close)
        .bang()
        .lifecycle(Lifecycle::Close),
    ex(&["on[ly]"], windows::only).bang(),
    ex(&["sp[lit]"], windows::split_horizontal)
        .args(A::File)
        .anywhere(),
    ex(&["vs[plit]"], windows::split_vertical)
        .args(A::File)
        .anywhere(),
    ex(&["tabnew"], windows::tab_new).args(A::File).anywhere(),
    ex(&["tabe[dit]"], windows::tab_new)
        .args(A::File)
        .anywhere(),
    ex(&["tabn[ext]"], windows::tab_next).anywhere(),
    ex(&["tabp[revious]", "tabN[ext]"], windows::tab_previous).anywhere(),
    ex(&["tabfir[st]", "tabr[ewind]"], windows::tab_first),
    ex(&["tabl[ast]"], windows::tab_last),
    ex(&["tabc[lose]"], windows::tab_close).bang(),
    ex(&["tabo[nly]"], windows::tab_only).bang(),
    ex(&["tabs"], windows::tabs),
    ex(&["ls", "buffers", "files"], windows::list_buffers)
        .bang()
        .anywhere(),
    ex(&["b[uffer]"], windows::buffer).bang().args(A::Text),
    ex(&["bn[ext]"], windows::next_buffer).bang().anywhere(),
    ex(&["bp[revious]", "bN[ext]"], windows::previous_buffer)
        .bang()
        .anywhere(),
    ex(&["bd[elete]"], windows::delete_buffer)
        .bang()
        .lifecycle(Lifecycle::Delete),
    // ---- options, mappings, listings, configuration ----
    ex(&["se[t]"], options::set).args(A::Text).anywhere(),
    ex(&["unset"], options::unset).args(A::Text),
    ex(&["colo[rscheme]"], options::colorscheme).args(A::Text),
    ex(&["map"], |e, x| options::map(e, x, MapMode::All, false)).args(A::Text),
    ex(&["nm[ap]"], |e, x| {
        options::map(e, x, MapMode::Normal, false)
    })
    .args(A::Text),
    ex(&["im[ap]"], |e, x| {
        options::map(e, x, MapMode::Insert, false)
    })
    .args(A::Text),
    ex(&["vm[ap]", "xm[ap]"], |e, x| {
        options::map(e, x, MapMode::Visual, false)
    })
    .args(A::Text),
    ex(&["cm[ap]"], |e, x| {
        options::map(e, x, MapMode::Command, false)
    })
    .args(A::Text),
    ex(&["no[remap]"], |e, x| {
        options::map(e, x, MapMode::All, true)
    })
    .args(A::Text),
    ex(&["nn[oremap]"], |e, x| {
        options::map(e, x, MapMode::Normal, true)
    })
    .args(A::Text),
    ex(&["ino[remap]"], |e, x| {
        options::map(e, x, MapMode::Insert, true)
    })
    .args(A::Text),
    ex(&["vn[oremap]", "xn[oremap]"], |e, x| {
        options::map(e, x, MapMode::Visual, true)
    })
    .args(A::Text),
    ex(&["cno[remap]"], |e, x| {
        options::map(e, x, MapMode::Command, true)
    })
    .args(A::Text),
    ex(&["unm[ap]"], |e, x| options::unmap(e, x, MapMode::All)).args(A::Text),
    ex(&["nun[map]"], |e, x| options::unmap(e, x, MapMode::Normal)).args(A::Text),
    ex(&["iu[nmap]"], |e, x| options::unmap(e, x, MapMode::Insert)).args(A::Text),
    ex(&["vu[nmap]", "xu[nmap]"], |e, x| {
        options::unmap(e, x, MapMode::Visual)
    })
    .args(A::Text),
    ex(&["cu[nmap]"], |e, x| options::unmap(e, x, MapMode::Command)).args(A::Text),
    ex(&["mapc[lear]"], |e, _| options::mapclear(e, MapMode::All)),
    ex(&["nmapc[lear]"], |e, _| {
        options::mapclear(e, MapMode::Normal)
    }),
    ex(&["imapc[lear]"], |e, _| {
        options::mapclear(e, MapMode::Insert)
    }),
    ex(&["vmapc[lear]", "xmapc[lear]"], |e, _| {
        options::mapclear(e, MapMode::Visual)
    }),
    ex(&["cmapc[lear]"], |e, _| {
        options::mapclear(e, MapMode::Command)
    }),
    ex(&["noh[lsearch]"], options::nohlsearch),
    ex(&["reg[isters]", "di[splay]"], options::registers).args(A::Text),
    ex(&["marks"], options::marks).args(A::Text),
    ex(&["h[elp]"], options::help).args(A::Text),
    ex(&["blame"], options::blame),
    ex(&["lua"], options::lua).args(A::Rest),
    ex(&["luaf[ile]"], options::luafile).args(A::File),
    ex(&["so[urce]"], options::source).args(A::File),
    ex(&["reload", "ConfigReload"], options::reload),
    // ---- quickfix and project ----
    ex(&["mak[e]"], quickfix::make).bang().args(A::Text),
    ex(&["cope[n]"], quickfix::open),
    ex(&["ccl[ose]"], quickfix::close),
    ex(&["cn[ext]"], quickfix::next).bang(),
    ex(&["cp[revious]", "cN[ext]"], quickfix::previous).bang(),
    ex(&["cfir[st]", "cr[ewind]"], quickfix::first).bang(),
    ex(&["cla[st]"], quickfix::last).bang(),
    ex(&["cdo"], quickfix::quickfix_do).args(A::Rest),
    ex(&["cfdo"], quickfix::quickfix_do).args(A::Rest),
    ex(&["gr[ep]", "vim[grep]"], project::grep).args(A::Rest),
    ex(
        &["SearchReplace", "Sr", "ReplaceInFiles"],
        project::search_replace,
    )
    .args(A::Rest),
    ex(&["ReplaceApply"], project::replace_apply),
    ex(&["ReplaceUndo"], project::replace_undo),
    ex(&["Problems", "Diagnostics"], project::problems).args(A::Text),
    ex(&["Symbols", "WorkspaceSymbols"], project::symbols).args(A::Text),
    ex(
        &["Outline", "DocumentSymbols"],
        call!(open_outline_picker()),
    ),
    ex(
        &["Recent", "RecentFiles"],
        call!(open_recent_files_picker()),
    ),
    ex(&["Buffers"], call!(open_buffer_picker())),
    // ---- git ----
    ex(&["GitDiff", "gitdiff", "DiffReview"], git::diff_review).args(A::Text),
    ex(&["GitDiffLayout", "gitdifflayout"], git::diff_layout).args(A::Text),
    ex(&["GitDiffFile"], git::diff_file).args(A::File),
    ex(&["GitEdit"], git::edit).args(A::File),
    ex(&["GitShow"], git::show).args(A::Text),
    ex(&["GitFetch", "gitfetch"], call!(fetch_review_base())),
    ex(&["GitStatus", "Gstatus"], call!(open_git_status_picker())),
    ex(&["GitStage", "GitStageFile"], call!(git_stage_file())),
    ex(&["GitUnstage", "GitUnstageFile"], call!(git_unstage_file())),
    ex(&["GitStageHunk"], call!(git_stage_hunk())),
    ex(&["GitUnstageHunk"], call!(git_unstage_hunk())),
    ex(&["GitStageAll"], call!(git_stage_all())),
    ex(&["GitCommit", "Gcommit"], call!(open_commit_message(false))),
    ex(&["GitAmend"], call!(open_commit_message(true))),
    ex(&["GitLog", "GitFileLog"], call!(open_file_history_picker())),
    ex(&["GitLogAll"], call!(open_repo_history_picker())),
    ex(
        &["GitLineLog", "GitLineHistory"],
        call!(open_line_history_picker()),
    ),
    ex(&["ConflictNext"], call!(goto_conflict(true))),
    ex(&["ConflictPrev"], call!(goto_conflict(false))),
    ex(&["ConflictOurs"], call!(resolve_conflict(Resolution::Ours))),
    ex(
        &["ConflictTheirs"],
        call!(resolve_conflict(Resolution::Theirs)),
    ),
    ex(&["ConflictBoth"], call!(resolve_conflict(Resolution::Both))),
    ex(
        &["ConflictNone"],
        call!(resolve_conflict(Resolution::Neither)),
    ),
    // ---- language servers ----
    ex(&["LspInfo"], lsp::info),
    ex(&["LspStatus"], lsp::status),
    ex(&["LspLog"], lsp::log),
    ex(&["LspRestart"], lsp::restart).args(A::Text),
    ex(&["LspExec"], lsp::exec).args(A::Rest),
    ex(&["LspRename"], lsp::rename).args(A::Text),
    ex(
        &["LspReloadProject"],
        call!(lsp_execute_command("hyperion.reloadProject", Vec::new())),
    ),
    ex(&["LspInstall", "LspManager"], call!(open_lsp_manager())),
    ex(
        &["format", "Format"],
        call!("Formatting document...", request_format_document()),
    ),
    // ---- run, test, debug ----
    ex(
        &["Run", "RunCursor"],
        call!(launch_at_cursor(LaunchMode::Run)),
    ),
    ex(&["Debug"], call!(launch_at_cursor(LaunchMode::Debug))),
    ex(
        &["RunConfig", "RunPick"],
        call!(launch_pick_config(LaunchMode::Run)),
    ),
    ex(
        &["DebugConfig", "DebugPick"],
        call!(launch_pick_config(LaunchMode::Debug)),
    ),
    ex(&["RunLast", "DebugLast"], call!(launch_last())),
    ex(&["RunStop"], call!(launch_stop())),
    ex(&["RunConsole", "RunToggle"], call!(toggle_run_console())),
    ex(&["RunFocus"], call!(focus_run_console())),
    ex(&["RunPrev"], call!(run_console_mut().view_previous())),
    ex(&["RunNext"], call!(run_console_mut().view_next())),
    ex(&["RunEof"], call!(run_eof())),
    ex(&["RunClear"], call!(clear_run_console())),
    ex(&["RunJump"], launch::run_jump).args(A::Text),
    ex(&["RunInput"], launch::run_input).args(A::Rest),
    ex(
        &["CodeLens", "CodeLensRun"],
        call!(run_code_lens_at_cursor(LaunchMode::Run)),
    ),
    ex(
        &["CodeLensDebug"],
        call!(run_code_lens_at_cursor(LaunchMode::Debug)),
    ),
    ex(
        &["TestFile", "TF"],
        call!("Running tests for current file...", run_test_file()),
    ),
    ex(
        &["TestNearest", "TN"],
        call!("Running nearest test...", run_test_nearest()),
    ),
    ex(
        &["TestAll", "TA", "TestSuite", "TS"],
        call!("Running all tests...", run_test_all()),
    ),
    ex(
        &["TestDebug", "TD"],
        call!("Debugging nearest test...", debug_test_nearest()),
    ),
    ex(
        &["TestDebugFile", "TDF"],
        call!("Debugging tests of current file...", debug_test_file()),
    ),
    ex(
        &["TestLast", "TL"],
        call!("Re-running last test...", run_test_last()),
    ),
    ex(
        &["TestVisit", "TV"],
        call!("Visiting last-tested position...", test_visit()),
    ),
    ex(&["TestPanel", "TestToggle", "TP"], launch::test_panel),
    ex(&["TestOutput", "MakeOutput"], launch::test_output),
    ex(&["PanelSize"], launch::panel_size).args(A::Text),
    ex(&["debug"], debug::debug).args(A::Text),
    ex(&["eval"], debug::eval).args(A::Rest),
    ex(&["DebugPanel", "DebugFocus"], debug::focus_panel),
    ex(&["DebugWatch"], debug::watch).args(A::Rest),
    ex(&["DebugUnwatch"], debug::unwatch).args(A::Rest),
    ex(&["DebugBreakpoints"], debug::breakpoints).args(A::Text),
    ex(&["DebugException"], debug::exception).args(A::Text),
    ex(&["DebugExpand"], debug::expand).args(A::Text),
    ex(&["DebugLogpoint"], debug::logpoint).args(A::Rest),
    ex(&["DebugHitCount"], debug::hit_count).args(A::Text),
    ex(&["DebugCondition"], debug::condition).args(A::Rest),
    // ---- sessions, workflows, AI ----
    ex(&["ai"], session::ai).args(A::Text),
    ex(&["workflow"], session::workflow).args(A::Text),
    ex(&["session"], session::session).args(A::Text),
    ex(&["clearaedits"], session::clear_agent_edits),
    ex(&["browser"], session::browser),
];

/// The full form of a `:help`-style name: `quit` for `q[uit]`.
pub fn full_name(spec: &str) -> String {
    spec.replace(['[', ']'], "")
}

/// Whether `typed` is an accepted spelling of `spec`.
fn name_matches(spec: &str, typed: &str) -> bool {
    match spec.split_once('[') {
        Some((required, optional)) => {
            let optional = optional.trim_end_matches(']');
            typed.len() >= required.len()
                && typed.len() <= required.len() + optional.len()
                && typed.starts_with(required)
                && optional.starts_with(&typed[required.len()..])
        }
        None => spec == typed,
    }
}

/// Every command name in full, sorted, for tab completion: derived from
/// the table, so it cannot list a command that does not exist or miss one
/// that does.
pub fn command_names() -> &'static [String] {
    static NAMES: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
        let mut names: Vec<String> = COMMANDS
            .iter()
            .flat_map(|command| command.names.iter().map(|spec| full_name(spec)))
            .filter(|name| name.starts_with(|c: char| c.is_ascii_alphabetic()))
            .collect();
        names.sort();
        names.dedup();
        names
    });
    &NAMES
}

/// Resolve a typed command name.
pub fn lookup(typed: &str) -> Option<&'static ExCommand> {
    COMMANDS
        .iter()
        .find(|command| command.names.iter().any(|spec| name_matches(spec, typed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_notation_accepts_every_prefix_from_the_required_part() {
        assert!(name_matches("s[ubstitute]", "s"));
        assert!(name_matches("s[ubstitute]", "subst"));
        assert!(!name_matches("s[ubstitute]", "substitutes"));
        assert!(!name_matches("sor[t]", "so"));
        assert!(name_matches("t", "t"));
        assert!(!name_matches("t", "ta"));
    }

    /// Every spelling the three old dispatchers accepted (read from
    /// commands.rs, cmd_project.rs, cmd_buffer.rs, cmd_set.rs and
    /// editor/input/commands.rs before OV-00487), with the command it
    /// resolves to now. Bang forms (`q!`, `bd!`, ...) are the same entries.
    const OLD_SPELLINGS: &[(&str, &str)] = &[
        ("q", "quit"),
        ("quit", "quit"),
        ("cq", "cquit"),
        ("cquit", "cquit"),
        ("qa", "qall"),
        ("qall", "qall"),
        ("wa", "wall"),
        ("wall", "wall"),
        ("writeall", "wall"),
        ("w", "write"),
        ("write", "write"),
        ("wq", "wq"),
        ("format", "format"),
        ("Format", "format"),
        ("GitDiff", "GitDiff"),
        ("gitdiff", "GitDiff"),
        ("DiffReview", "GitDiff"),
        ("GitDiffLayout", "GitDiffLayout"),
        ("gitdifflayout", "GitDiffLayout"),
        ("unset", "unset"),
        ("GitFetch", "GitFetch"),
        ("gitfetch", "GitFetch"),
        ("LspInfo", "LspInfo"),
        ("LspReloadProject", "LspReloadProject"),
        ("LspExec", "LspExec"),
        ("LspRestart", "LspRestart"),
        ("LspStatus", "LspStatus"),
        ("LspLog", "LspLog"),
        ("LspRename", "LspRename"),
        ("Run", "Run"),
        ("RunCursor", "Run"),
        ("Debug", "Debug"),
        ("RunConfig", "RunConfig"),
        ("RunPick", "RunConfig"),
        ("DebugConfig", "DebugConfig"),
        ("DebugPick", "DebugConfig"),
        ("RunLast", "RunLast"),
        ("DebugLast", "RunLast"),
        ("RunStop", "RunStop"),
        ("RunConsole", "RunConsole"),
        ("RunToggle", "RunConsole"),
        ("RunFocus", "RunFocus"),
        ("CodeLens", "CodeLens"),
        ("CodeLensRun", "CodeLens"),
        ("CodeLensDebug", "CodeLensDebug"),
        ("RunPrev", "RunPrev"),
        ("RunNext", "RunNext"),
        ("RunJump", "RunJump"),
        ("RunEof", "RunEof"),
        ("RunInput", "RunInput"),
        ("RunClear", "RunClear"),
        ("TestFile", "TestFile"),
        ("TF", "TestFile"),
        ("TestNearest", "TestNearest"),
        ("TN", "TestNearest"),
        ("TestAll", "TestAll"),
        ("TA", "TestAll"),
        ("TestSuite", "TestAll"),
        ("TS", "TestAll"),
        ("TestDebug", "TestDebug"),
        ("TD", "TestDebug"),
        ("TestDebugFile", "TestDebugFile"),
        ("TDF", "TestDebugFile"),
        ("TestLast", "TestLast"),
        ("TL", "TestLast"),
        ("TestVisit", "TestVisit"),
        ("TV", "TestVisit"),
        ("PanelSize", "PanelSize"),
        ("TestPanel", "TestPanel"),
        ("TestToggle", "TestPanel"),
        ("TP", "TestPanel"),
        ("TestOutput", "TestOutput"),
        ("MakeOutput", "TestOutput"),
        ("make", "make"),
        ("copen", "copen"),
        ("cclose", "cclose"),
        ("ccl", "cclose"),
        ("cnext", "cnext"),
        ("cn", "cnext"),
        ("cprev", "cprevious"),
        ("cp", "cprevious"),
        ("cprevious", "cprevious"),
        ("cfirst", "cfirst"),
        ("cfir", "cfirst"),
        ("clast", "clast"),
        ("cla", "clast"),
        ("tabnew", "tabnew"),
        ("tabe", "tabedit"),
        ("tabedit", "tabedit"),
        ("tabnext", "tabnext"),
        ("tabn", "tabnext"),
        ("tabprev", "tabprevious"),
        ("tabp", "tabprevious"),
        ("tabprevious", "tabprevious"),
        ("tabfirst", "tabfirst"),
        ("tabfir", "tabfirst"),
        ("tablast", "tablast"),
        ("tabl", "tablast"),
        ("tabclose", "tabclose"),
        ("tabc", "tabclose"),
        ("ls", "ls"),
        ("buffers", "ls"),
        ("files", "ls"),
        ("bnext", "bnext"),
        ("bn", "bnext"),
        ("bprev", "bprevious"),
        ("bp", "bprevious"),
        ("bprevious", "bprevious"),
        ("bd", "bdelete"),
        ("bdelete", "bdelete"),
        ("tabonly", "tabonly"),
        ("tabo", "tabonly"),
        ("blame", "blame"),
        ("noh", "nohlsearch"),
        ("nohlsearch", "nohlsearch"),
        ("reg", "registers"),
        ("registers", "registers"),
        ("j", "join"),
        ("join", "join"),
        ("recover", "recover"),
        ("rec", "recover"),
        ("checktime", "checktime"),
        ("marks", "marks"),
        ("tabs", "tabs"),
        ("clearaedits", "clearaedits"),
        ("lua", "lua"),
        ("luafile", "luafile"),
        ("colorscheme", "colorscheme"),
        ("colo", "colorscheme"),
        ("set", "set"),
        ("se", "set"),
        ("sp", "split"),
        ("split", "split"),
        ("vsp", "vsplit"),
        ("vsplit", "vsplit"),
        ("only", "only"),
        ("on", "only"),
        ("ConfigReload", "reload"),
        ("reload", "reload"),
        ("source", "source"),
        ("so", "source"),
        ("e", "edit"),
        ("edit", "edit"),
        ("help", "help"),
        ("map", "map"),
        ("nmap", "nmap"),
        ("imap", "imap"),
        ("vmap", "vmap"),
        ("xmap", "vmap"),
        ("cmap", "cmap"),
        ("noremap", "noremap"),
        ("nnoremap", "nnoremap"),
        ("inoremap", "inoremap"),
        ("vnoremap", "vnoremap"),
        ("xnoremap", "vnoremap"),
        ("cnoremap", "cnoremap"),
        ("unmap", "unmap"),
        ("nunmap", "nunmap"),
        ("iunmap", "iunmap"),
        ("vunmap", "vunmap"),
        ("xunmap", "vunmap"),
        ("cunmap", "cunmap"),
        ("mapclear", "mapclear"),
        ("nmapclear", "nmapclear"),
        ("imapclear", "imapclear"),
        ("vmapclear", "vmapclear"),
        ("xmapclear", "vmapclear"),
        ("cmapclear", "cmapclear"),
        ("ai", "ai"),
        ("workflow", "workflow"),
        ("session", "session"),
        ("debug", "debug"),
        ("eval", "eval"),
        ("DebugPanel", "DebugPanel"),
        ("DebugFocus", "DebugPanel"),
        ("DebugWatch", "DebugWatch"),
        ("DebugUnwatch", "DebugUnwatch"),
        ("DebugBreakpoints", "DebugBreakpoints"),
        ("DebugException", "DebugException"),
        ("DebugExpand", "DebugExpand"),
        ("DebugLogpoint", "DebugLogpoint"),
        ("DebugHitCount", "DebugHitCount"),
        ("DebugCondition", "DebugCondition"),
        ("f", "file"),
        ("file", "file"),
        ("pwd", "pwd"),
        ("cd", "cd"),
        ("lcd", "cd"),
        ("LspInstall", "LspInstall"),
        ("LspManager", "LspInstall"),
        ("u", "undo"),
        ("undo", "undo"),
        ("red", "redo"),
        ("redo", "redo"),
        ("browser", "browser"),
        // cmd_project.rs
        ("SearchReplace", "SearchReplace"),
        ("Sr", "SearchReplace"),
        ("ReplaceInFiles", "SearchReplace"),
        ("ReplaceApply", "ReplaceApply"),
        ("ReplaceUndo", "ReplaceUndo"),
        ("Recent", "Recent"),
        ("RecentFiles", "Recent"),
        ("Buffers", "Buffers"),
        ("Problems", "Problems"),
        ("Diagnostics", "Problems"),
        ("Outline", "Outline"),
        ("DocumentSymbols", "Outline"),
        ("Symbols", "Symbols"),
        ("WorkspaceSymbols", "Symbols"),
        ("GitStatus", "GitStatus"),
        ("Gstatus", "GitStatus"),
        ("GitStage", "GitStage"),
        ("GitStageFile", "GitStage"),
        ("GitUnstage", "GitUnstage"),
        ("GitUnstageFile", "GitUnstage"),
        ("GitStageHunk", "GitStageHunk"),
        ("GitUnstageHunk", "GitUnstageHunk"),
        ("GitStageAll", "GitStageAll"),
        ("GitCommit", "GitCommit"),
        ("Gcommit", "GitCommit"),
        ("GitAmend", "GitAmend"),
        ("GitLog", "GitLog"),
        ("GitFileLog", "GitLog"),
        ("GitLogAll", "GitLogAll"),
        ("GitLineLog", "GitLineLog"),
        ("GitLineHistory", "GitLineLog"),
        ("GitDiffFile", "GitDiffFile"),
        ("GitEdit", "GitEdit"),
        ("GitShow", "GitShow"),
        ("ConflictNext", "ConflictNext"),
        ("ConflictPrev", "ConflictPrev"),
        ("ConflictOurs", "ConflictOurs"),
        ("ConflictTheirs", "ConflictTheirs"),
        ("ConflictBoth", "ConflictBoth"),
        ("ConflictNone", "ConflictNone"),
        ("update", "update"),
        ("up", "update"),
        ("grep", "grep"),
        ("gr", "grep"),
        ("vimgrep", "grep"),
        ("vim", "grep"),
        // editor/input/commands.rs
        ("cdo", "cdo"),
        ("cfdo", "cfdo"),
        ("terminal", "terminal"),
        ("term", "terminal"),
        ("shell", "terminal"),
        ("d", "delete"),
        ("delete", "delete"),
        ("y", "yank"),
        ("yank", "yank"),
        ("sort", "sort"),
        ("t", "t"),
        ("copy", "t"),
        ("m", "move"),
        ("move", "move"),
        ("s", "substitute"),
        ("g", "global"),
        ("v", "vglobal"),
        ("r", "read"),
        ("read", "read"),
        ("b", "buffer"),
        ("buffer", "buffer"),
        ("!", "!"),
    ];

    #[test]
    fn every_old_spelling_resolves_to_its_command() {
        for (typed, canonical) in OLD_SPELLINGS {
            let command = lookup(typed).unwrap_or_else(|| panic!("{typed:?} is unknown"));
            assert_eq!(&command.name(), canonical, "{typed:?}");
        }
    }

    /// vim's abbreviations, checked with nvim's `fullcommand()` (NVIM
    /// v0.12.2): every spelling from the required part to the full name.
    #[test]
    fn vim_abbreviations_match_fullcommand() {
        for (typed, canonical) in [
            ("qu", "quit"),
            ("quita", "qall"),
            ("wr", "write"),
            ("xi", "xit"),
            ("exi", "xit"),
            ("xa", "wqall"),
            ("wqa", "wqall"),
            ("sav", "saveas"),
            ("de", "delete"),
            ("ya", "yank"),
            ("jo", "join"),
            ("sor", "sort"),
            ("co", "t"),
            ("mo", "move"),
            ("su", "substitute"),
            ("gl", "global"),
            ("vg", "vglobal"),
            ("norm", "normal"),
            ("un", "undo"),
            ("re", "read"),
            ("tabN", "tabprevious"),
            ("tabr", "tabfirst"),
            ("bN", "bprevious"),
            ("cN", "cprevious"),
            ("cr", "cfirst"),
            ("vs", "vsplit"),
            ("clo", "close"),
            ("cope", "copen"),
            ("mak", "make"),
            ("nn", "nnoremap"),
            ("no", "noremap"),
            ("ino", "inoremap"),
            ("unm", "unmap"),
            ("mapc", "mapclear"),
            ("nohl", "nohlsearch"),
            ("di", "registers"),
            ("fi", "file"),
            ("pw", "pwd"),
            ("lc", "cd"),
            ("checkt", "checktime"),
            ("ter", "terminal"),
            ("h", "help"),
            ("p", "print"),
            ("luaf", "luafile"),
        ] {
            let command = lookup(typed).unwrap_or_else(|| panic!("{typed:?} is unknown"));
            assert_eq!(command.name(), canonical, "{typed:?}");
        }
        // Shorter than the required part: not a command (or another one).
        assert!(lookup("sa").is_none());
        assert_eq!(lookup("s").unwrap().name(), "substitute");
        assert_eq!(lookup("so").unwrap().name(), "source");
    }

    #[test]
    fn completion_lists_exactly_the_table() {
        let names = command_names();
        for command in COMMANDS {
            for spec in command
                .names
                .iter()
                .filter(|spec| !matches!(**spec, "" | "!"))
            {
                assert!(names.contains(&full_name(spec)), "{spec} missing");
            }
        }
        for name in names {
            assert!(lookup(name).is_some(), "{name} does not resolve");
        }
        // Offered by the old hand-written list without a handler.
        for dead in [
            "delmarks",
            "history",
            "messages",
            "highlight",
            "tabmove",
            "unlet",
        ] {
            assert!(!names.iter().any(|name| name == dead), "{dead}");
        }
    }

    #[test]
    fn every_name_is_unambiguous() {
        // Two entries accepting the same spelling would make the table order
        // decide silently.
        for command in COMMANDS {
            for spec in command.names {
                let full = full_name(spec);
                let required = spec.split_once('[').map_or(spec.len(), |(r, _)| r.len());
                for len in required..=full.len() {
                    let typed = &full[..len];
                    let owners: Vec<String> = COMMANDS
                        .iter()
                        .filter(|other| other.names.iter().any(|s| name_matches(s, typed)))
                        .map(ExCommand::name)
                        .collect();
                    assert_eq!(owners.len(), 1, "{typed:?} resolves to {owners:?}");
                }
            }
        }
    }
}
