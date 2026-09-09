// Example binary: unwrap keeps the demo focused on the widget being shown.
#![allow(clippy::unwrap_used)]

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ferrowl_ui::{
    AlternateScreen, Border,
    state::{
        DiffViewState, DiffViewStateBuilder, FileStatus, FileTreeOutcome, FileTreeState,
        FileTreeStateBuilder, SuggestInputState, SuggestInputStateBuilder,
    },
    traits::{HandleEvents, SetFocus, Suggestion, SuggestionProvider},
    widgets::{
        DiffViewBuilder, FileTreeBuilder, InputFieldBuilder, SuggestInput, SuggestInputBuilder,
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
};
use std::{io::Stdout, process::Command, time::Duration};

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
    fn next(self) -> Self {
        match self {
            Focus::BaseInput => Focus::BranchInput,
            Focus::BranchInput => Focus::FileBrowser,
            Focus::FileBrowser => Focus::DiffViewer,
            Focus::DiffViewer => Focus::BaseInput,
        }
    }

    fn previous(self) -> Self {
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
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).split(rows[1]);
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
/// guessed at.
fn parse_name_status(out: &str) -> Vec<(String, Option<FileStatus>)> {
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
            Some((path.to_string(), status))
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
    paths: Vec<(String, Option<FileStatus>)>,
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
        match (self.git)(&["diff", "--name-status", "--no-renames", &range]) {
            Ok(out) => {
                self.paths = parse_name_status(&out);
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

    let tree_widget = FileTreeBuilder::default().build().unwrap();
    f.render_stateful_widget(&tree_widget, panes.browser, &mut model.tree);

    let diff_widget = DiffViewBuilder::default().build().unwrap();
    f.render_stateful_widget(&diff_widget, panes.diff, &mut model.diff);

    base_widget.render_overlay(panes.base, f.buffer_mut(), &mut model.base);
    branch_widget.render_overlay(panes.branch, f.buffer_mut(), &mut model.branch);

    if let Some(err) = &model.error {
        ratatui::widgets::Widget::render(
            ratatui::text::Text::from(err.as_str()),
            panes.diff,
            f.buffer_mut(),
        );
    }
}

/// Routes one key event to whichever pane currently holds focus, cycling focus on
/// `Tab`/`Shift+Tab` and reloading whenever a ref input's text changed.
fn handle_key(model: &mut Model, modifiers: KeyModifiers, code: KeyCode) {
    match (modifiers, code) {
        (KeyModifiers::NONE, KeyCode::Tab) => {
            set_focus(model, model.focus.next());
            return;
        }
        (_, KeyCode::BackTab) | (KeyModifiers::SHIFT, KeyCode::Tab) => {
            set_focus(model, model.focus.previous());
            return;
        }
        _ => {}
    }

    // `q` quits only while an input pane is not focused: with an input focused, `q`
    // must stay ordinary typed text for branch names containing it.
    let input_focused = model.focus == Focus::BaseInput || model.focus == Focus::BranchInput;
    if !input_focused && modifiers == KeyModifiers::NONE && code == KeyCode::Char('q') {
        model.done = true;
        return;
    }

    match model.focus {
        Focus::BaseInput => {
            let before = model.base.input().clone();
            model.base.handle_events(modifiers, code);
            if &before != model.base.input() {
                model.reload();
            }
        }
        Focus::BranchInput => {
            let before = model.branch.input().clone();
            model.branch.handle_events(modifiers, code);
            if &before != model.branch.input() {
                model.reload();
            }
        }
        Focus::FileBrowser => {
            if let Some(FileTreeOutcome::Activated(path)) = model.tree.handle_key(modifiers, code) {
                model.show_file(&path);
            }
        }
        Focus::DiffViewer => {
            model.diff.handle_events(modifiers, code);
        }
    }
}

fn set_focus(model: &mut Model, focus: Focus) {
    model.base.set_focused(focus == Focus::BaseInput);
    model.branch.set_focused(focus == Focus::BranchInput);
    model.tree.set_focused(focus == Focus::FileBrowser);
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
        let base = base.to_string();
        let branch = branch.to_string();
        let name_status = name_status.to_string();
        let whole_diff = whole_diff.to_string();
        let shows: Vec<(String, String)> = shows
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect();
        Box::new(move |args: &[&str]| -> Result<String, String> {
            let range = format!("{base}...{branch}");
            match args {
                ["for-each-ref", ..] => Ok(format!("{base}\n{branch}\n")),
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
    /// left/right.
    fn ut_panes_put_two_inputs_above_a_browser_and_a_diff_pane() {
        let p = panes(Rect::new(0, 0, 100, 40));
        assert_eq!(p.base.y, p.branch.y);
        assert!(p.base.x < p.branch.x);
        assert_eq!(p.browser.y, p.diff.y);
        assert!(p.browser.y > p.base.y);
        assert!(p.browser.x < p.diff.x);
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
        let mut model = Model::new(fixture("main", "main", "", ""));
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
            vec![("src/a.rs".to_string(), Some(FileStatus::Modified))]
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
            vec![("src/b.rs".to_string(), Some(FileStatus::Added))]
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
        );
        assert_eq!(
            parsed,
            vec![
                ("src/new.rs".to_string(), Some(FileStatus::Added)),
                ("src/old.rs".to_string(), Some(FileStatus::Removed)),
                ("src/changed.rs".to_string(), Some(FileStatus::Modified)),
                ("src/moved.rs".to_string(), None),
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
    /// the example falls back to the hunk-only view (UI-R-259) without an error.
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

    fn row_count(diff: &DiffViewState) -> usize {
        let mut i = 0;
        while diff.row(i).is_some() {
            i += 1;
        }
        i
    }
}
