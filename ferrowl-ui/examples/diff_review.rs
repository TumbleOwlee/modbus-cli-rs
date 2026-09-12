// Example binary: unwrap keeps the demo focused on the widget being shown.
#![allow(clippy::unwrap_used)]

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ferrowl_ui::{
    AlternateScreen, Border, EventResult,
    state::{
        DiffViewState, DiffViewStateBuilder, FileStatus, FileTreeBadge, FileTreeEntry,
        FileTreeOutcome, FileTreeState, FileTreeStateBuilder, SuggestInputState,
        SuggestInputStateBuilder,
    },
    traits::{HandleEvents, SetFocus, Suggestion, SuggestionProvider},
    widgets::{
        DiffViewBuilder, FileTreeBuilder, InputFieldBuilder, SuggestInput, SuggestInputBuilder,
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::Style,
    widgets::{Block, Widget},
};
use std::{collections::HashMap, io::Stdout, process::Command, time::Duration};

/// Suggests local git branches by prefix, from a fixed snapshot taken once at startup:
/// local branches don't change mid-session, so re-querying on every keystroke would
/// only add a git subprocess spawn per key with no observable benefit.
#[derive(Debug, Clone)]
struct BranchProvider(Vec<String>);

impl SuggestionProvider for BranchProvider {
    fn suggest(&self, input: &str) -> Vec<Suggestion> {
        self.0
            .iter()
            .filter(|b| b.starts_with(input))
            .map(|b| Suggestion {
                value: b.clone(),
                label: b.clone(),
                partial: false,
            })
            .collect()
    }
}

/// Which of the example's four panes currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    BaseInput,
    BranchInput,
    FileBrowser,
    DiffViewer,
}

impl Focus {
    /// Cycles forward through base, branch, browser, diff, wrapping; the browser and
    /// diff panes join the order only once `has_diff` reports a diff has been produced.
    fn next(self, has_diff: bool) -> Self {
        if !has_diff {
            return match self {
                Focus::BaseInput => Focus::BranchInput,
                Focus::BranchInput | Focus::FileBrowser | Focus::DiffViewer => Focus::BaseInput,
            };
        }
        match self {
            Focus::BaseInput => Focus::BranchInput,
            Focus::BranchInput => Focus::FileBrowser,
            Focus::FileBrowser => Focus::DiffViewer,
            Focus::DiffViewer => Focus::BaseInput,
        }
    }

    fn previous(self, has_diff: bool) -> Self {
        if !has_diff {
            return match self {
                Focus::BranchInput => Focus::BaseInput,
                Focus::BaseInput | Focus::FileBrowser | Focus::DiffViewer => Focus::BranchInput,
            };
        }
        match self {
            Focus::BaseInput => Focus::DiffViewer,
            Focus::BranchInput => Focus::BaseInput,
            Focus::FileBrowser => Focus::BranchInput,
            Focus::DiffViewer => Focus::FileBrowser,
        }
    }
}

/// Rects for the example's four panes: two inputs on top, a browser and a diff
/// viewer split beneath them.
struct Panes {
    base: Rect,
    branch: Rect,
    browser: Rect,
    diff: Rect,
}

fn panes(area: Rect) -> Panes {
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(area);
    let top =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[0]);
    let bottom =
        Layout::horizontal([Constraint::Percentage(20), Constraint::Percentage(80)]).split(rows[1]);
    Panes {
        base: top[0],
        branch: top[1],
        browser: bottom[0],
        diff: bottom[1],
    }
}

/// Parses `git for-each-ref --format=%(refname:short) refs/heads` output into branch names.
fn parse_branches(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Parses `git diff --name-status --no-renames` output into paths with their change
/// status. An unrecognized status letter is treated as unchanged (`None`) rather than
/// guessed at. `counts` is a per-path `(added, removed)` line-count table from `git diff
/// --numstat`; a matching path gets a `"+<added> -<removed>"` badge, a path with no match
/// (e.g. a binary file) gets none.
fn parse_name_status(out: &str, counts: &HashMap<String, (u64, u64)>) -> Vec<FileTreeEntry> {
    out.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\t');
            let code = parts.next()?.trim();
            let path = parts.next()?.trim();
            if path.is_empty() {
                return None;
            }
            let status = match code.chars().next()? {
                'A' => Some(FileStatus::Added),
                'D' => Some(FileStatus::Removed),
                'M' => Some(FileStatus::Modified),
                _ => None,
            };
            let mut entry = FileTreeEntry::new(path);
            if let Some(status) = status {
                entry = entry.with_status(status);
            }
            if let Some((added, removed)) = counts.get(path) {
                entry = entry.with_badge(FileTreeBadge::new(
                    format!("+{added} -{removed}"),
                    Style::default(),
                ));
            }
            Some(entry)
        })
        .collect()
}

/// Parses `git diff --numstat` output into per-path added/removed line counts. A binary
/// file reports `-` for both counts; those paths are skipped rather than badged with a
/// bogus number.
fn parse_numstat(out: &str) -> HashMap<String, (u64, u64)> {
    out.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let added = parts.next()?.trim();
            let removed = parts.next()?.trim();
            let path = parts.next()?.trim();
            if path.is_empty() {
                return None;
            }
            let added: u64 = added.parse().ok()?;
            let removed: u64 = removed.parse().ok()?;
            Some((path.to_string(), (added, removed)))
        })
        .collect()
}

/// Shorthand for the git seam's function type: takes the argv, returns stdout or an
/// error message.
type GitFn = Box<dyn Fn(&[&str]) -> Result<String, String>>;

/// Runs a real `git` subprocess, the seam's default implementation outside tests.
fn run_git(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

struct Model {
    git: GitFn,
    base: SuggestInputState<BranchProvider>,
    branch: SuggestInputState<BranchProvider>,
    tree: FileTreeState,
    diff: DiffViewState,
    focus: Focus,
    /// The paths last passed to the file tree's `set_paths`, kept here because the
    /// tree's own row list is crate-private: this is the only way anything outside
    /// `ferrowl-ui` (including this example's tests) can observe what it was given.
    paths: Vec<FileTreeEntry>,
    /// The branch names last fetched from the git seam, kept here for the same reason as
    /// `paths`: nothing outside this module can otherwise observe what the suggestion
    /// providers were built with. Read only by this file's own tests.
    #[allow(dead_code)]
    branches: Vec<String>,
    error: Option<String>,
    done: bool,
}

impl Model {
    fn new(git: GitFn) -> Self {
        let (branches, error) =
            match git(&["for-each-ref", "--format=%(refname:short)", "refs/heads"]) {
                Ok(out) => (parse_branches(&out), None),
                Err(e) => (Vec::new(), Some(e)),
            };
        let base = SuggestInputStateBuilder::default()
            .provider(BranchProvider(branches.clone()))
            .build()
            .unwrap();
        let branch = SuggestInputStateBuilder::default()
            .provider(BranchProvider(branches.clone()))
            .build()
            .unwrap();
        let tree = FileTreeStateBuilder::default().build().unwrap();
        let diff = DiffViewStateBuilder::default().build_with_diff("").unwrap();
        Self {
            git,
            base,
            branch,
            tree,
            diff,
            focus: Focus::BaseInput,
            paths: Vec::new(),
            branches,
            error,
            done: false,
        }
    }

    fn has_diff(&self) -> bool {
        self.diff.row(0).is_some()
    }

    fn set_diff(&mut self, text: &str) {
        self.diff = DiffViewStateBuilder::default()
            .build_with_diff(text)
            .unwrap();
    }

    /// Rebuilds both the browser's paths and the shown diff from the current base and
    /// branch inputs. A blank input or matching refs short-circuit without touching the
    /// git seam, since the result is guaranteed empty either way.
    fn reload(&mut self) {
        self.error = None;
        let base = self.base.input().clone();
        let branch = self.branch.input().clone();
        if base.is_empty() || branch.is_empty() || base == branch {
            self.paths = Vec::new();
            self.tree.set_paths(&[]);
            self.set_diff("");
            return;
        }
        let range = format!("{base}...{branch}");
        let counts = match (self.git)(&["diff", "--numstat", &range]) {
            Ok(out) => parse_numstat(&out),
            Err(_) => HashMap::new(),
        };
        match (self.git)(&["diff", "--name-status", "--no-renames", &range]) {
            Ok(out) => {
                self.paths = parse_name_status(&out, &counts);
                self.tree.set_paths(&self.paths);
            }
            Err(e) => {
                self.paths = Vec::new();
                self.tree.set_paths(&[]);
                self.error = Some(e);
            }
        }
        match (self.git)(&["diff", &range]) {
            Ok(out) => self.set_diff(&out),
            Err(e) => {
                self.set_diff("");
                self.error = Some(e);
            }
        }
    }

    /// Shows one file's diff between the current base and branch, with the whole file's
    /// changes marked in place when its full new-side text is available. A file deleted
    /// on the branch has no new-side text to fetch; `git show` fails and the view falls
    /// back to hunk-only (UI-R-259).
    fn show_file(&mut self, path: &str) {
        let base = self.base.input().clone();
        let branch = self.branch.input().clone();
        let range = format!("{base}...{branch}");
        match (self.git)(&["diff", &range, "--", path]) {
            Ok(diff) => match (self.git)(&["show", &format!("{branch}:{path}")]) {
                Ok(new_text) => {
                    self.diff = DiffViewStateBuilder::default()
                        .build_with_diff_and_file(&diff, &new_text)
                        .unwrap();
                }
                Err(_) => self.set_diff(&diff),
            },
            Err(e) => {
                self.set_diff("");
                self.error = Some(e);
            }
        }
    }
}

fn ui(f: &mut Frame, model: &mut Model) {
    let panes = panes(f.area());

    let base_widget: SuggestInput<String, BranchProvider> = SuggestInputBuilder::default()
        .input_field(
            InputFieldBuilder::default()
                .title(Some("Base".into()))
                .border(Border::Full(Margin::new(1, 0)))
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    f.render_stateful_widget(&base_widget, panes.base, &mut model.base);

    let branch_widget: SuggestInput<String, BranchProvider> = SuggestInputBuilder::default()
        .input_field(
            InputFieldBuilder::default()
                .title(Some("Branch".into()))
                .border(Border::Full(Margin::new(1, 0)))
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    f.render_stateful_widget(&branch_widget, panes.branch, &mut model.branch);

    let tree_widget = FileTreeBuilder::default()
        .title(Some("File Tree".into()))
        .border(Border::Full(Margin {
            horizontal: 1,
            vertical: 0,
        }))
        .build()
        .unwrap();
    f.render_stateful_widget(&tree_widget, panes.browser, &mut model.tree);

    if let Some(err) = &model.error {
        let block = Block::bordered().title("Diff View");
        let inner = block.inner(panes.diff);
        block.render(panes.diff, f.buffer_mut());
        let diff_area = inner.inner(Margin {
            vertical: 0,
            horizontal: 1,
        });
        ratatui::widgets::Widget::render(
            ratatui::text::Text::from(err.as_str()),
            diff_area,
            f.buffer_mut(),
        );
    } else {
        let diff_widget = DiffViewBuilder::default()
            .border(Border::Full(Margin {
                horizontal: 1,
                vertical: 0,
            }))
            .build()
            .unwrap();
        f.render_stateful_widget(&diff_widget, panes.diff, &mut model.diff);
    }
    base_widget.render_overlay(f.area(), f.buffer_mut(), &mut model.base);
    branch_widget.render_overlay(f.area(), f.buffer_mut(), &mut model.branch);
}

/// Routes one key event to whichever pane currently holds focus, cycling focus on
/// `Tab`/`Shift+Tab` and reloading whenever a ref input's text changed.
fn handle_key(model: &mut Model, modifiers: KeyModifiers, code: KeyCode) {
    match (modifiers, code) {
        (KeyModifiers::NONE, KeyCode::Tab) => {
            let has_diff = model.has_diff();
            set_focus(model, model.focus.next(has_diff));
            return;
        }
        (_, KeyCode::BackTab) | (KeyModifiers::SHIFT, KeyCode::Tab) => {
            let has_diff = model.has_diff();
            set_focus(model, model.focus.previous(has_diff));
            return;
        }
        (KeyModifiers::NONE, KeyCode::Esc) => {
            if !dispatch_to_focused(model, modifiers, code) {
                model.done = true;
            }
            return;
        }
        _ => {}
    }

    dispatch_to_focused(model, modifiers, code);
}

/// Sends one key to whichever pane holds focus, and reports whether that pane
/// consumed it: an input closing its suggestion dropdown, the file tree activating or
/// toggling a node, or the diff widget acting on the key all count as consumed, so
/// `Esc` (UI-R-223) can be layered above the example's own exit handling.
fn dispatch_to_focused(model: &mut Model, modifiers: KeyModifiers, code: KeyCode) -> bool {
    match model.focus {
        Focus::BaseInput => {
            let before = model.base.input().clone();
            let result = model.base.handle_events(modifiers, code);
            if &before != model.base.input() {
                model.reload();
            }
            matches!(result, EventResult::Consumed)
        }
        Focus::BranchInput => {
            let before = model.branch.input().clone();
            let result = model.branch.handle_events(modifiers, code);
            if &before != model.branch.input() {
                model.reload();
            }
            matches!(result, EventResult::Consumed)
        }
        Focus::FileBrowser => {
            let outcome = model.tree.handle_key(modifiers, code);
            if let Some(FileTreeOutcome::Activated(path)) = &outcome {
                model.show_file(path);
            }
            outcome.is_some()
        }
        Focus::DiffViewer => {
            matches!(
                model.diff.handle_events(modifiers, code),
                EventResult::Consumed
            )
        }
    }
}

fn set_focus(model: &mut Model, focus: Focus) {
    model.base.set_focused(focus == Focus::BaseInput);
    model.branch.set_focused(focus == Focus::BranchInput);
    model.tree.set_focused(focus == Focus::FileBrowser);
    model.diff.set_focused(focus == Focus::DiffViewer);
    model.focus = focus;
}

fn main() {
    let mut screen: AlternateScreen<Stdout> =
        AlternateScreen::new().expect("Failed to create alternate screen.");

    let mut model = Model::new(Box::new(run_git));
    set_focus(&mut model, Focus::BaseInput);

    while !model.done {
        screen.draw(|f| ui(f, &mut model)).unwrap();

        if event::poll(Duration::from_millis(50)).unwrap()
            && let Event::Key(key) = event::read().unwrap()
            && key.kind == KeyEventKind::Press
        {
            handle_key(&mut model, key.modifiers, key.code);
        }
    }

    drop(screen);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(base: &str, branch: &str, name_status: &str, whole_diff: &str) -> GitFn {
        fixture_with_show(base, branch, name_status, whole_diff, &[])
    }

    /// Like `fixture`, plus a table of `(path, full new-side text)` pairs that `git show
    /// <branch>:<path>` resolves to; any path not listed fails, standing in for a file
    /// deleted on the branch.
    fn fixture_with_show(
        base: &str,
        branch: &str,
        name_status: &str,
        whole_diff: &str,
        shows: &[(&str, &str)],
    ) -> GitFn {
        fixture_with_numstat(base, branch, name_status, whole_diff, "", shows)
    }

    /// Like `fixture_with_show`, plus the `git diff --numstat` output the reload's
    /// line-count lookup resolves to.
    fn fixture_with_numstat(
        base: &str,
        branch: &str,
        name_status: &str,
        whole_diff: &str,
        numstat: &str,
        shows: &[(&str, &str)],
    ) -> GitFn {
        let base = base.to_string();
        let branch = branch.to_string();
        let name_status = name_status.to_string();
        let whole_diff = whole_diff.to_string();
        let numstat = numstat.to_string();
        let shows: Vec<(String, String)> = shows
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect();
        Box::new(move |args: &[&str]| -> Result<String, String> {
            let range = format!("{base}...{branch}");
            match args {
                ["for-each-ref", ..] => Ok(format!("{base}\n{branch}\n")),
                ["diff", "--numstat", r] if *r == range => Ok(numstat.clone()),
                ["diff", "--name-status", "--no-renames", r] if *r == range => {
                    Ok(name_status.clone())
                }
                ["diff", r] if *r == range => Ok(whole_diff.clone()),
                ["diff", r, "--", path] if *r == range => Ok(format!("diff for {path}\n")),
                ["show", rev_path] => {
                    let prefix = format!("{branch}:");
                    let path = rev_path
                        .strip_prefix(&prefix)
                        .ok_or_else(|| format!("unexpected show arg: {rev_path}"))?;
                    shows
                        .iter()
                        .find(|(p, _)| p == path)
                        .map(|(_, text)| text.clone())
                        .ok_or_else(|| format!("path {path} does not exist on {branch}"))
                }
                _ => Err(format!("unexpected git args: {args:?}")),
            }
        })
    }

    #[test]
    /// The top row holds two inputs side by side, above a browser and a diff pane split
    /// left/right, with the diff pane given four times the browser's width (20/80).
    fn ut_panes_put_two_inputs_above_a_browser_and_a_diff_pane() {
        let p = panes(Rect::new(0, 0, 100, 40));
        assert_eq!(p.base.y, p.branch.y);
        assert!(p.base.x < p.branch.x);
        assert_eq!(p.browser.y, p.diff.y);
        assert!(p.browser.y > p.base.y);
        assert!(p.browser.x < p.diff.x);
        assert_eq!(p.browser.width, 20);
        assert_eq!(p.diff.width, 80);
    }

    fn rendered_text(model: &mut Model) -> String {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 40)).unwrap();
        term.draw(|f| ui(f, model)).unwrap();
        let buf = term.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    /// The file browser pane renders inside a titled, bordered block.
    fn ut_file_tree_pane_renders_with_a_title_and_border() {
        let mut model = Model::new(fixture("main", "main", "", ""));
        let text = rendered_text(&mut model);
        assert!(
            text.contains("File Tree"),
            "missing file tree title:\n{text}"
        );
        assert!(text.contains('│'), "missing vertical border:\n{text}");
    }

    #[test]
    /// When the git seam fails, the diff pane renders the error inside a titled,
    /// bordered block instead of the diff viewer.
    fn ut_error_renders_inside_a_titled_bordered_diff_pane() {
        let git: GitFn = Box::new(|_: &[&str]| Err("not a git repository".to_string()));
        let mut model = Model::new(git);
        let text = rendered_text(&mut model);
        assert!(
            text.contains("Diff View"),
            "missing diff view title:\n{text}"
        );
        assert!(
            text.contains("not a git repository"),
            "missing error text:\n{text}"
        );
    }

    #[test]
    /// The branch suggestion list comes from the git seam's branch output and filters
    /// candidates by prefix.
    fn ut_branch_list_comes_from_git_output_and_filters_by_prefix() {
        let provider = BranchProvider(parse_branches("main\nfeature/a\nfeature/b\n"));
        let suggestions = provider.suggest("feature/");
        assert_eq!(
            suggestions
                .iter()
                .map(|s| s.value.clone())
                .collect::<Vec<_>>(),
            vec!["feature/a".to_string(), "feature/b".to_string()]
        );
    }

    #[test]
    /// `Tab` cycles base, branch, browser, diff forward with wrap; `Shift+Tab` cycles the
    /// same order in reverse with wrap.
    fn ut_tab_cycles_the_four_panes_forward_and_shift_tab_backward_with_wrap() {
        let mut model = Model::new(fixture(
            "main",
            "feature",
            "M\tsrc/a.rs\n",
            "diff --git a/src/a.rs b/src/a.rs\n@@ -1 +1 @@\n-old\n+new\n",
        ));
        model.base.set_input("main".to_string());
        model.branch.set_input("feature".to_string());
        model.reload();
        assert!(model.has_diff());
        set_focus(&mut model, Focus::BaseInput);
        let forward = [
            Focus::BaseInput,
            Focus::BranchInput,
            Focus::FileBrowser,
            Focus::DiffViewer,
            Focus::BaseInput,
        ];
        for want in forward.iter().skip(1) {
            handle_key(&mut model, KeyModifiers::NONE, KeyCode::Tab);
            assert_eq!(model.focus, *want);
        }
        for want in forward.iter().rev().skip(1) {
            handle_key(&mut model, KeyModifiers::NONE, KeyCode::BackTab);
            assert_eq!(model.focus, *want);
        }
    }

    #[test]
    /// Before a valid base+branch selection has produced a diff, `Tab` and `Shift+Tab`
    /// skip the file browser and diff viewer, cycling base and branch alone.
    fn ut_tab_skips_the_browser_and_diff_pane_until_a_diff_exists() {
        let mut model = Model::new(fixture("main", "main", "", ""));
        assert!(!model.has_diff());
        set_focus(&mut model, Focus::BaseInput);
        handle_key(&mut model, KeyModifiers::NONE, KeyCode::Tab);
        assert_eq!(model.focus, Focus::BranchInput);
        handle_key(&mut model, KeyModifiers::NONE, KeyCode::Tab);
        assert_eq!(model.focus, Focus::BaseInput);
        handle_key(&mut model, KeyModifiers::NONE, KeyCode::BackTab);
        assert_eq!(model.focus, Focus::BranchInput);
        handle_key(&mut model, KeyModifiers::NONE, KeyCode::BackTab);
        assert_eq!(model.focus, Focus::BaseInput);
    }

    #[test]
    /// Activating a file shows that file's diff, and changing a ref rebuilds both the
    /// browser's paths and the shown diff.
    fn ut_activating_a_file_shows_its_diff_and_changing_a_ref_rebuilds_both() {
        let mut model = Model::new(fixture(
            "main",
            "feature",
            "M\tsrc/a.rs\n",
            "@@ -1,2 +1,2 @@\n-a\n-b\n+a\n+b\n",
        ));
        model.base.set_input("main".to_string());
        model.branch.set_input("feature".to_string());
        model.reload();
        assert_eq!(
            model.paths,
            vec![FileTreeEntry::new("src/a.rs").with_status(FileStatus::Modified)]
        );
        let whole_row_count = row_count(&model.diff);
        assert_eq!(whole_row_count, 3);

        model.show_file("src/a.rs");
        assert_eq!(row_count(&model.diff), 1);

        model.git = fixture(
            "main",
            "other",
            "A\tsrc/b.rs\n",
            "diff --git a/src/b.rs b/src/b.rs\n",
        );
        model.branch.set_input("other".to_string());
        model.reload();
        assert_eq!(
            model.paths,
            vec![FileTreeEntry::new("src/b.rs").with_status(FileStatus::Added)]
        );
        assert_eq!(row_count(&model.diff), 1);
    }

    #[test]
    /// Before any file is activated, the diff pane shows the real diff between the
    /// selected base and branch.
    fn ut_diff_pane_shows_the_whole_ref_diff_before_any_file_is_activated() {
        let mut model = Model::new(fixture(
            "main",
            "feature",
            "M\tsrc/a.rs\n",
            "diff --git a/src/a.rs b/src/a.rs\n@@ -1 +1 @@\n-old\n+new\n",
        ));
        model.base.set_input("main".to_string());
        model.branch.set_input("feature".to_string());
        model.reload();
        let row = model.diff.row(2).unwrap();
        assert_eq!(row.old_line, Some(1));
        assert_eq!(row.new_line, Some(1));
    }

    #[test]
    /// `git diff --name-status` output maps to paths and change statuses, an
    /// unrecognized status letter mapping to `None`.
    fn ut_name_status_output_maps_to_paths_and_change_statuses() {
        let parsed = parse_name_status(
            "A\tsrc/new.rs\nD\tsrc/old.rs\nM\tsrc/changed.rs\nR100\tsrc/moved.rs\n",
            &HashMap::new(),
        );
        assert_eq!(
            parsed,
            vec![
                FileTreeEntry::new("src/new.rs").with_status(FileStatus::Added),
                FileTreeEntry::new("src/old.rs").with_status(FileStatus::Removed),
                FileTreeEntry::new("src/changed.rs").with_status(FileStatus::Modified),
                FileTreeEntry::new("src/moved.rs"),
            ]
        );
    }

    #[test]
    /// `git diff --numstat` output maps each path to an `(added, removed)` pair; a binary
    /// file's `-`/`-` counts are skipped rather than parsed as zero.
    fn ut_numstat_output_maps_paths_to_added_and_removed_line_counts() {
        let parsed =
            parse_numstat("12\t3\tsrc/changed.rs\n0\t7\tsrc/old.rs\n-\t-\tsrc/image.png\n");
        assert_eq!(
            parsed,
            HashMap::from([
                ("src/changed.rs".to_string(), (12, 3)),
                ("src/old.rs".to_string(), (0, 7)),
            ])
        );
    }

    #[test]
    /// Reloading attaches a `"+<added> -<removed>"` badge built from `--numstat` to the
    /// matching path's entry, and leaves a path absent from `--numstat` (e.g. binary)
    /// without a badge.
    fn ut_reload_badges_each_path_with_its_added_and_removed_line_counts() {
        let mut model = Model::new(fixture_with_numstat(
            "main",
            "feature",
            "M\tsrc/changed.rs\nA\tsrc/image.png\n",
            "",
            "12\t3\tsrc/changed.rs\n-\t-\tsrc/image.png\n",
            &[],
        ));
        model.base.set_input("main".to_string());
        model.branch.set_input("feature".to_string());
        model.reload();
        assert_eq!(
            model.paths,
            vec![
                FileTreeEntry::new("src/changed.rs")
                    .with_status(FileStatus::Modified)
                    .with_badge(FileTreeBadge::new("+12 -3", Style::default())),
                FileTreeEntry::new("src/image.png").with_status(FileStatus::Added),
            ]
        );
    }

    #[test]
    /// When the git seam fails, the branch lists and browser stay empty and the example
    /// records the failure as text instead of exiting or panicking.
    fn ut_git_failure_leaves_the_branch_lists_empty_and_shows_the_error() {
        let git: GitFn = Box::new(|_: &[&str]| Err("not a git repository".to_string()));
        let model = Model::new(git);
        assert!(model.paths.is_empty());
        assert!(model.branches.is_empty());
        assert_eq!(model.error.as_deref(), Some("not a git repository"));
    }

    #[test]
    /// Base and branch set to the same ref yield no paths and an empty diff, with no
    /// error recorded.
    fn ut_same_ref_yields_no_paths_and_an_empty_diff_view_without_an_error() {
        let mut model = Model::new(fixture(
            "main",
            "main",
            "M\tsrc/a.rs\n",
            "should not be used",
        ));
        model.base.set_input("main".to_string());
        model.branch.set_input("main".to_string());
        model.reload();
        assert!(model.paths.is_empty());
        assert!(model.error.is_none());
        assert_eq!(row_count(&model.diff), 0);
    }

    #[test]
    /// Activating a file fetches its full new-side text through the git seam and builds
    /// the viewer with it, so the rows cover lines outside the changed hunk.
    fn ut_activated_file_is_shown_from_its_full_new_side_text() {
        let mut model = Model::new(fixture_with_show(
            "main",
            "feature",
            "M\tsrc/a.rs\n",
            "",
            &[("src/a.rs", "before\nA\nafter\n")],
        ));
        model.base.set_input("main".to_string());
        model.branch.set_input("feature".to_string());
        model.show_file("src/a.rs");
        assert!(model.error.is_none());
        assert!(
            row_count(&model.diff) >= 3,
            "the full new-side text has three lines; the hunk-only diff alone has one"
        );
    }

    #[test]
    /// A file deleted on the selected branch has no new-side text; `git show` fails and
    /// the example falls back to the hunk-only view without an error.
    fn ut_file_missing_on_the_branch_falls_back_to_hunk_only_without_an_error() {
        let mut model = Model::new(fixture_with_show(
            "main",
            "feature",
            "D\tsrc/gone.rs\n",
            "",
            &[],
        ));
        model.base.set_input("main".to_string());
        model.branch.set_input("feature".to_string());
        model.show_file("src/gone.rs");
        assert!(model.error.is_none());
        assert_eq!(row_count(&model.diff), 1);
    }

    #[test]
    /// `Esc` exits the example when the focused pane has nothing open to dismiss.
    fn ut_escape_exits_when_nothing_is_open_to_dismiss() {
        let mut model = Model::new(fixture("main", "main", "", ""));
        set_focus(&mut model, Focus::BranchInput);
        assert!(!model.branch.suggestions_open());
        handle_key(&mut model, KeyModifiers::NONE, KeyCode::Esc);
        assert!(model.done);
    }

    #[test]
    /// `Esc` closes an open suggestion dropdown instead of quitting, and a second `Esc`
    /// with nothing left open then exits.
    fn ut_escape_dismisses_an_open_suggestion_dropdown_before_quitting() {
        let mut model = Model::new(fixture("main", "feature", "", ""));
        set_focus(&mut model, Focus::BaseInput);
        handle_key(&mut model, KeyModifiers::NONE, KeyCode::Char('m'));
        assert!(model.base.suggestions_open());
        handle_key(&mut model, KeyModifiers::NONE, KeyCode::Esc);
        assert!(!model.done);
        assert!(!model.base.suggestions_open());
        handle_key(&mut model, KeyModifiers::NONE, KeyCode::Esc);
        assert!(model.done);
    }

    #[test]
    /// An open suggestion popup is clamped to the frame, not to the field it belongs to,
    /// so it drops below the field instead of covering the field's own titled block.
    fn ut_open_suggestions_do_not_cover_the_input_title() {
        let mut model = Model::new(fixture("main", "feature", "", ""));
        set_focus(&mut model, Focus::BaseInput);
        handle_key(&mut model, KeyModifiers::NONE, KeyCode::Char('m'));
        assert!(model.base.suggestions_open());
        let text = rendered_text(&mut model);
        assert!(
            text.contains("feature") || text.contains("main"),
            "missing suggestion row:\n{text}"
        );
        assert!(text.contains("Base"), "missing base title:\n{text}");
    }

    fn row_count(diff: &DiffViewState) -> usize {
        let mut i = 0;
        while diff.row(i).is_some() {
            i += 1;
        }
        i
    }
}
