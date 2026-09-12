use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use ratatui::style::Style;

use crate::EventResult;
use crate::style::SyntaxTheme;
use crate::traits::{HandleEvents, IsFocus, SetFocus};

/// The marker and style a file tree draws for a node carrying this status. `theme` is the
/// widget's syntax theme; an implementation is free to ignore it.
pub trait FileTreeStatus: Clone {
    fn marker(&self) -> String;
    fn style(&self, theme: &SyntaxTheme) -> Style;
}

/// A file node's change status: drawn as a leading marker and styled with the syntax
/// theme's added/removed/meta styles. Public because the caller sets it per path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Removed,
    Modified,
}

impl FileTreeStatus for FileStatus {
    fn marker(&self) -> String {
        match self {
            FileStatus::Added => "+".to_string(),
            FileStatus::Removed => "-".to_string(),
            FileStatus::Modified => "~".to_string(),
        }
    }

    fn style(&self, theme: &SyntaxTheme) -> Style {
        match self {
            FileStatus::Added => theme.added,
            FileStatus::Removed => theme.removed,
            FileStatus::Modified => theme.meta,
        }
    }
}

/// The text and optional style a file tree draws for a node's badge; `style` reporting
/// `None` falls back to the row's own styling.
pub trait FileTreeBadge: Clone {
    fn text(&self) -> String;
    fn style(&self) -> Option<Style>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoBadge;

impl FileTreeBadge for NoBadge {
    fn text(&self) -> String {
        String::new()
    }

    fn style(&self) -> Option<Style> {
        None
    }
}

/// One path plus what the caller attaches to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTreeEntry<S = FileStatus, B = NoBadge> {
    path: String,
    status: Option<S>,
    badge: Option<B>,
}

impl<S, B> FileTreeEntry<S, B> {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            status: None,
            badge: None,
        }
    }

    pub fn with_status(mut self, status: S) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_badge(mut self, badge: B) -> Self {
        self.badge = Some(badge);
        self
    }
}

/// A node of the tree built from the paths [`FileTreeState`] is constructed with. Crate-private
/// like `DiffRow` in `diff_view.rs`: no `api-contract.md` row exposes the tree itself,
/// only the path list going in and the selected path coming out.
#[derive(Debug, Clone)]
pub(crate) enum TreeNode<S> {
    Dir {
        name: String,
        children: Vec<TreeNode<S>>,
        expanded: bool,
    },
    File {
        name: String,
        status: Option<S>,
    },
}

/// One row of `FileTreeState::visible_rows()`'s depth-first walk. Crate-private: the
/// widget that renders this state is its only other caller, and no `api-contract.md` row
/// exposes the row list.
#[derive(Debug, Clone)]
pub(crate) struct VisibleRow<S, B> {
    pub(crate) depth: usize,
    pub(crate) path: String,
    pub(crate) is_dir: bool,
    pub(crate) expanded: bool,
    // Read by the widget that renders this state, not by anything in this module.
    #[allow(dead_code)]
    pub(crate) name: String,
    #[allow(dead_code)]
    pub(crate) status: Option<S>,
    #[allow(dead_code)]
    pub(crate) badge: Option<B>,
}

/// Outcome of a key offered to [`FileTreeState`] via [`handle_key`](FileTreeState::handle_key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileTreeOutcome {
    /// `Enter` on a file (UI-R-242): the file's full path.
    Activated(String),
    /// `Enter` on a directory flipped its expansion (UI-R-242).
    Toggled,
    /// Every other key this state handles.
    Consumed,
}

/// Splits each path on `/`, creating the directory nodes its components imply and hanging
/// the file under the last one (UI-R-234); a path with no `/` becomes a file node directly
/// under the root (UI-E-104). Every directory created is `expanded: true` (UI-R-235).
fn build_tree<S: FileTreeStatus, B>(entries: &[FileTreeEntry<S, B>]) -> Vec<TreeNode<S>> {
    let mut root: Vec<TreeNode<S>> = Vec::new();
    for entry in entries {
        let path = &entry.path;
        let status = &entry.status;
        let mut components: Vec<&str> = path.split('/').collect();
        let file_name = components.pop().unwrap_or_default();
        let mut siblings = &mut root;
        for dir_name in components {
            let idx = match siblings.iter().position(|n| match n {
                TreeNode::Dir { name, .. } => name == dir_name,
                TreeNode::File { .. } => false,
            }) {
                Some(idx) => idx,
                None => {
                    siblings.push(TreeNode::Dir {
                        name: dir_name.to_string(),
                        children: Vec::new(),
                        expanded: true,
                    });
                    siblings.len() - 1
                }
            };
            siblings = match &mut siblings[idx] {
                TreeNode::Dir { children, .. } => children,
                TreeNode::File { .. } => unreachable!(),
            };
        }
        siblings.push(TreeNode::File {
            name: file_name.to_string(),
            status: status.clone(),
        });
    }
    root
}

fn push_visible<S: FileTreeStatus, B: FileTreeBadge>(
    nodes: &[TreeNode<S>],
    depth: usize,
    prefix: &str,
    badges: &HashMap<String, B>,
    out: &mut Vec<VisibleRow<S, B>>,
) {
    let mut dirs: Vec<&TreeNode<S>> = nodes
        .iter()
        .filter(|n| matches!(n, TreeNode::Dir { .. }))
        .collect();
    dirs.sort_by_key(|n| match n {
        TreeNode::Dir { name, .. } => name.clone(),
        TreeNode::File { .. } => unreachable!(),
    });
    let mut files: Vec<&TreeNode<S>> = nodes
        .iter()
        .filter(|n| matches!(n, TreeNode::File { .. }))
        .collect();
    files.sort_by_key(|n| match n {
        TreeNode::File { name, .. } => name.clone(),
        TreeNode::Dir { .. } => unreachable!(),
    });

    for node in dirs.into_iter().chain(files) {
        match node {
            TreeNode::Dir {
                name,
                children,
                expanded,
            } => {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                out.push(VisibleRow {
                    depth,
                    path: path.clone(),
                    is_dir: true,
                    expanded: *expanded,
                    name: name.clone(),
                    status: None,
                    badge: None,
                });
                if *expanded {
                    push_visible(children, depth + 1, &path, badges, out);
                }
            }
            TreeNode::File { name, status } => {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                let badge = badges.get(&path).cloned();
                out.push(VisibleRow {
                    depth,
                    path,
                    is_dir: false,
                    expanded: false,
                    name: name.clone(),
                    status: status.clone(),
                    badge,
                });
            }
        }
    }
}

fn walk_mut<S>(nodes: &mut [TreeNode<S>], f: &mut impl FnMut(&mut bool)) {
    for node in nodes {
        if let TreeNode::Dir {
            children, expanded, ..
        } = node
        {
            f(expanded);
            walk_mut(children, f);
        }
    }
}

/// State of a [`FileTree`](crate::widgets::FileTree): a tree built from a path list plus
/// the current selection and viewport.
#[derive(Builder, Debug, Clone)]
pub struct FileTreeState<S = FileStatus, B = NoBadge> {
    #[builder(setter(custom), default = "Vec::new()")]
    root: Vec<TreeNode<S>>,
    #[builder(setter(custom), default = "HashMap::new()")]
    badges: HashMap<String, B>,
    #[builder(setter(skip), default = "0")]
    selected: usize,
    #[builder(setter(skip), default = "0")]
    scroll_offset: usize,
    #[builder(default = "1")]
    visible_height: usize,
    #[builder(setter(skip), default = "None")]
    pending: Option<char>,
    // UI-R-246's only observable effect (border style) lives in the widget; mutated
    // through `SetFocus::set_focused` below, not a generated field setter.
    #[builder(default = "true")]
    focused: bool,
}

/// Collects the badge each entry carries into a path-keyed map, shared by
/// `FileTreeStateBuilder::paths` and `FileTreeState::set_paths`.
fn badges_from<S, B: Clone>(entries: &[FileTreeEntry<S, B>]) -> HashMap<String, B> {
    entries
        .iter()
        .filter_map(|entry| {
            entry
                .badge
                .as_ref()
                .map(|b| (entry.path.clone(), b.clone()))
        })
        .collect()
}

impl<S: FileTreeStatus, B: FileTreeBadge> FileTreeStateBuilder<S, B> {
    /// Entries plus their optional status and badge, routed through `build_tree` for the
    /// tree and collected into the badge map.
    pub fn paths(&mut self, entries: Vec<FileTreeEntry<S, B>>) -> &mut Self {
        self.badges = Some(badges_from(&entries));
        self.root = Some(build_tree(&entries));
        self
    }
}

impl Default for FileTreeState<FileStatus, NoBadge> {
    fn default() -> Self {
        FileTreeStateBuilder::default()
            .build()
            .expect("FileTreeStateBuilder fields all default")
    }
}

impl<S: FileTreeStatus, B: FileTreeBadge> FileTreeState<S, B> {
    /// UI-R-234 — rebuilds the tree from a fresh path list, routed through `build_tree`
    /// like the builder's `paths` setter; the selection is clamped to the new row count.
    pub fn set_paths(&mut self, entries: &[FileTreeEntry<S, B>]) {
        self.badges = badges_from(entries);
        self.root = build_tree(entries);
        let rows = self.visible_rows();
        self.selected = self.selected.min(rows.len().saturating_sub(1));
        self.scroll_offset = 0;
        self.ensure_visible();
    }

    /// Sets, replaces or (with `None`) clears one path's badge; the path need not name a
    /// file node. Selection and expansion are untouched.
    pub fn set_badge(&mut self, path: &str, badge: Option<B>) {
        match badge {
            Some(b) => {
                self.badges.insert(path.to_string(), b);
            }
            None => {
                self.badges.remove(path);
            }
        }
    }

    /// UI-R-235 — expands every directory in the tree.
    pub fn expand_all(&mut self) {
        walk_mut(&mut self.root, &mut |expanded| *expanded = true);
    }

    /// UI-R-235 — collapses every directory in the tree.
    pub fn collapse_all(&mut self) {
        walk_mut(&mut self.root, &mut |expanded| *expanded = false);
    }

    /// UI-R-236, UI-R-237 — a depth-first walk descending only into expanded
    /// directories, directories before files, each group ordered by name.
    pub(crate) fn visible_rows(&self) -> Vec<VisibleRow<S, B>> {
        let mut out = Vec::new();
        push_visible(&self.root, 0, "", &self.badges, &mut out);
        out
    }

    /// UI-R-243, UI-E-103 — the selected node's full path, `None` on an empty tree.
    pub fn selected_path(&self) -> Option<String> {
        self.visible_rows()
            .into_iter()
            .nth(self.selected)
            .map(|r| r.path)
    }

    /// UI-R-243, UI-E-103 — whether the selected node is a directory, `None` on an empty
    /// tree.
    pub fn selected_is_dir(&self) -> Option<bool> {
        self.visible_rows().get(self.selected).map(|r| r.is_dir)
    }

    // Read by the widget that renders this state, not by anything in this module outside
    // tests.
    #[allow(dead_code)]
    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    // Read by the widget that renders this state, not by anything in this module outside
    // tests.
    #[allow(dead_code)]
    pub(crate) fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    // Written by the widget that renders this state, not by anything in this module
    // outside tests.
    #[allow(dead_code)]
    pub(crate) fn set_visible_height(&mut self, height: usize) {
        self.visible_height = height;
    }

    /// UI-R-245 — keeps the selected row inside the last-rendered visible window, the
    /// same remembered-height scheme as the code editor's `page_move`/`handle_readonly_nav`.
    fn ensure_visible(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + self.visible_height {
            self.scroll_offset = self.selected + 1 - self.visible_height;
        }
    }

    /// Mirrors `code_input_field.rs`'s `page_move`: moves by at least one row, clamped at
    /// the first and last visible row.
    fn page_move(&mut self, down: bool, rows: usize) {
        let rows = rows.max(1);
        let last = self.visible_rows().len().saturating_sub(1);
        self.selected = if down {
            (self.selected + rows).min(last)
        } else {
            self.selected.saturating_sub(rows)
        };
        self.ensure_visible();
    }

    /// UI-R-240 — expand a collapsed directory, descend into an already-expanded one, do
    /// nothing on a file.
    fn expand_or_descend(&mut self) {
        let rows = self.visible_rows();
        let Some(row) = rows.get(self.selected) else {
            return;
        };
        if !row.is_dir {
            return;
        }
        if !row.expanded {
            self.set_expanded(&row.path, true);
        } else if self.selected + 1 < rows.len() && rows[self.selected + 1].depth > row.depth {
            self.selected += 1;
        }
    }

    /// UI-R-241, UI-E-105 — collapse an expanded directory, otherwise ascend to the
    /// parent, doing nothing at the top level.
    fn collapse_or_ascend(&mut self) {
        let rows = self.visible_rows();
        let Some(row) = rows.get(self.selected) else {
            return;
        };
        if row.is_dir && row.expanded {
            self.set_expanded(&row.path, false);
            return;
        }
        let depth = row.depth;
        if depth == 0 {
            return;
        }
        for i in (0..self.selected).rev() {
            if rows[i].depth < depth {
                self.selected = i;
                break;
            }
        }
    }

    fn set_expanded(&mut self, path: &str, expanded: bool) {
        fn go<S>(nodes: &mut [TreeNode<S>], prefix: &str, path: &str, expanded: bool) -> bool {
            for node in nodes {
                if let TreeNode::Dir {
                    name,
                    children,
                    expanded: node_expanded,
                } = node
                {
                    let node_path = if prefix.is_empty() {
                        name.clone()
                    } else {
                        format!("{prefix}/{name}")
                    };
                    if node_path == path {
                        *node_expanded = expanded;
                        return true;
                    }
                    if go(children, &node_path, path, expanded) {
                        return true;
                    }
                }
            }
            false
        }
        go(&mut self.root, "", path, expanded);
    }

    /// `None` for a key this state does not handle. UI-R-239, UI-R-240, UI-R-241, UI-R-242,
    /// UI-R-245, UI-E-105.
    pub fn handle_key(
        &mut self,
        modifiers: KeyModifiers,
        code: KeyCode,
    ) -> Option<FileTreeOutcome> {
        if modifiers == KeyModifiers::NONE && code == KeyCode::Char('g') {
            if self.pending == Some('g') {
                self.pending = None;
                self.selected = 0;
                self.ensure_visible();
            } else {
                self.pending = Some('g');
            }
            return Some(FileTreeOutcome::Consumed);
        }
        self.pending = None;

        match (modifiers, code) {
            (KeyModifiers::NONE, KeyCode::Char('j') | KeyCode::Down) => {
                let last = self.visible_rows().len().saturating_sub(1);
                self.selected = (self.selected + 1).min(last);
                self.ensure_visible();
                Some(FileTreeOutcome::Consumed)
            }
            (KeyModifiers::NONE, KeyCode::Char('k') | KeyCode::Up) => {
                self.selected = self.selected.saturating_sub(1);
                self.ensure_visible();
                Some(FileTreeOutcome::Consumed)
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('G')) => {
                self.selected = self.visible_rows().len().saturating_sub(1);
                self.ensure_visible();
                Some(FileTreeOutcome::Consumed)
            }
            (KeyModifiers::NONE, KeyCode::Char('l') | KeyCode::Right) => {
                self.expand_or_descend();
                self.ensure_visible();
                Some(FileTreeOutcome::Consumed)
            }
            (KeyModifiers::NONE, KeyCode::Char('h') | KeyCode::Left) => {
                self.collapse_or_ascend();
                self.ensure_visible();
                Some(FileTreeOutcome::Consumed)
            }
            (KeyModifiers::NONE, KeyCode::PageDown) => {
                self.page_move(true, self.visible_height);
                Some(FileTreeOutcome::Consumed)
            }
            (KeyModifiers::NONE, KeyCode::PageUp) => {
                self.page_move(false, self.visible_height);
                Some(FileTreeOutcome::Consumed)
            }
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
                self.page_move(true, (self.visible_height / 2).max(1));
                Some(FileTreeOutcome::Consumed)
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                self.page_move(false, (self.visible_height / 2).max(1));
                Some(FileTreeOutcome::Consumed)
            }
            (KeyModifiers::NONE, KeyCode::Enter) => {
                let rows = self.visible_rows();
                match rows.get(self.selected) {
                    None => Some(FileTreeOutcome::Consumed),
                    Some(row) if row.is_dir => {
                        let path = row.path.clone();
                        let expanded = row.expanded;
                        self.set_expanded(&path, !expanded);
                        Some(FileTreeOutcome::Toggled)
                    }
                    Some(row) => Some(FileTreeOutcome::Activated(row.path.clone())),
                }
            }
            _ => None,
        }
    }
}

impl<S: FileTreeStatus, B: FileTreeBadge> HandleEvents for FileTreeState<S, B> {
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        match self.handle_key(modifiers, code) {
            Some(_) => EventResult::Consumed,
            None => EventResult::Unhandled(modifiers, code),
        }
    }
}

impl<S: FileTreeStatus, B: FileTreeBadge> SetFocus for FileTreeState<S, B> {
    fn set_focused(&mut self, focus: bool) {
        self.focused = focus;
    }
}

impl<S: FileTreeStatus, B: FileTreeBadge> IsFocus for FileTreeState<S, B> {
    fn is_focused(&self) -> bool {
        self.focused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths<B>(list: &[(&str, Option<FileStatus>)]) -> Vec<FileTreeEntry<FileStatus, B>> {
        list.iter()
            .map(|(p, s)| {
                let mut e = FileTreeEntry::new(*p);
                if let Some(s) = s {
                    e = e.with_status(*s);
                }
                e
            })
            .collect()
    }

    fn tree(list: &[(&str, Option<FileStatus>)]) -> FileTreeState {
        FileTreeStateBuilder::default()
            .paths(paths(list))
            .build()
            .unwrap()
    }

    #[test]
    /// UI-R-234 — the tree is derived from path components: a nested path creates the
    /// directory nodes its components imply.
    fn ut_tree_is_derived_from_path_components() {
        let s = tree(&[("src/main.rs", None)]);
        let rows = s.visible_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "src");
        assert!(rows[0].is_dir);
        assert_eq!(rows[1].name, "main.rs");
        assert_eq!(rows[1].path, "src/main.rs");
        assert_eq!(rows[1].depth, 1);
    }

    #[test]
    /// UI-E-104 — a path with no directory component is a file node directly under the
    /// root.
    fn ut_path_without_a_directory_component_is_a_root_level_file() {
        let s = tree(&[("README.md", None)]);
        let rows = s.visible_rows();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].is_dir);
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[0].path, "README.md");
    }

    #[test]
    /// UI-E-103 — an empty path list has no rows and no selected node.
    fn ut_empty_path_list_has_no_rows_and_no_selected_node() {
        let s = tree(&[]);
        assert!(s.visible_rows().is_empty());
        assert_eq!(s.selected_path(), None);
        assert_eq!(s.selected_is_dir(), None);
    }

    #[test]
    /// UI-R-235 — directories start expanded; expand_all/collapse_all flip them.
    fn ut_directories_start_expanded_and_expand_all_collapse_all_flip_them() {
        let mut s = tree(&[("a/b.rs", None)]);
        assert_eq!(s.visible_rows().len(), 2);
        s.collapse_all();
        assert_eq!(s.visible_rows().len(), 1);
        s.expand_all();
        assert_eq!(s.visible_rows().len(), 2);
    }

    #[test]
    /// UI-R-236 — a collapsed directory contributes no visible rows.
    fn ut_collapsed_directory_contributes_no_visible_rows() {
        let mut s = tree(&[("a/b.rs", None), ("c.rs", None)]);
        s.collapse_all();
        s.expand_all();
        s.set_expanded("a", false);
        let rows = s.visible_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "a");
        assert_eq!(rows[1].name, "c.rs");
    }

    #[test]
    /// UI-R-237 — siblings order directories first, then files, each group by name.
    fn ut_siblings_order_directories_first_then_files_each_by_name() {
        let s = tree(&[
            ("z.rs", None),
            ("b/x.rs", None),
            ("a.rs", None),
            ("a/y.rs", None),
        ]);
        let rows = s.visible_rows();
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "y.rs", "b", "x.rs", "a.rs", "z.rs"]);
    }

    #[test]
    /// UI-R-243 — selected_path/selected_is_dir follow the selection.
    fn ut_selected_path_and_directory_query_follow_the_selection() {
        let mut s = tree(&[("a/b.rs", None), ("c.rs", None)]);
        assert_eq!(s.selected_path().as_deref(), Some("a"));
        assert_eq!(s.selected_is_dir(), Some(true));
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.selected_path().as_deref(), Some("a/b.rs"));
        assert_eq!(s.selected_is_dir(), Some(false));
    }

    #[test]
    /// UI-R-239 — j/k/gg/G move the selection and clamp at the ends.
    fn ut_j_k_gg_and_g_move_the_selection_and_clamp() {
        let mut s = tree(&[("a.rs", None), ("b.rs", None), ("c.rs", None)]);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.selected(), 1);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('j'));
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.selected(), 2);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('k'));
        assert_eq!(s.selected(), 1);
        s.handle_key(KeyModifiers::SHIFT, KeyCode::Char('G'));
        assert_eq!(s.selected(), 2);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('g'));
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('g'));
        assert_eq!(s.selected(), 0);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('k'));
        assert_eq!(s.selected(), 0);
    }

    #[test]
    /// UI-R-240 — l expands a collapsed directory, then descends into it, and does
    /// nothing on a file.
    fn ut_l_expands_then_descends_and_does_nothing_on_a_file() {
        let mut s = tree(&[("a/b.rs", None)]);
        s.collapse_all();
        assert_eq!(s.visible_rows().len(), 1);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('l'));
        assert_eq!(s.visible_rows().len(), 2);
        assert_eq!(s.selected(), 0);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('l'));
        assert_eq!(s.selected(), 1);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('l'));
        assert_eq!(s.selected(), 1);
    }

    #[test]
    /// UI-R-241, UI-E-105 — h collapses an expanded directory, otherwise ascends to the
    /// parent, and does nothing at the top level.
    fn ut_h_collapses_then_ascends_and_stops_at_a_top_level_node() {
        let mut s = tree(&[("a/b.rs", None)]);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.selected(), 1);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('h'));
        assert_eq!(s.selected(), 0);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('h'));
        assert_eq!(s.visible_rows().len(), 1);
        assert_eq!(s.selected(), 0);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('h'));
        assert_eq!(s.selected(), 0);
    }

    #[test]
    /// UI-R-242 — Enter toggles a directory's expansion and activates a file with its
    /// full path.
    fn ut_enter_toggles_a_directory_and_activates_a_file_with_its_full_path() {
        let mut s = tree(&[("a/b.rs", None)]);
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Enter);
        assert_eq!(outcome, Some(FileTreeOutcome::Toggled));
        assert_eq!(s.visible_rows().len(), 1);
        s.handle_key(KeyModifiers::NONE, KeyCode::Enter);
        assert_eq!(s.visible_rows().len(), 2);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('j'));
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Enter);
        assert_eq!(
            outcome,
            Some(FileTreeOutcome::Activated("a/b.rs".to_string()))
        );
    }

    #[test]
    /// UI-R-242 — an unhandled key reports `None`; every key this state handles reports
    /// an outcome.
    fn ut_unhandled_key_reports_none_and_consumed_keys_report_an_outcome() {
        let mut s = tree(&[("a.rs", None)]);
        assert_eq!(s.handle_key(KeyModifiers::NONE, KeyCode::Char('x')), None);
        assert_eq!(
            s.handle_key(KeyModifiers::NONE, KeyCode::Char('j')),
            Some(FileTreeOutcome::Consumed)
        );
        assert!(matches!(
            s.handle_events(KeyModifiers::NONE, KeyCode::Char('x')),
            EventResult::Unhandled(KeyModifiers::NONE, KeyCode::Char('x'))
        ));
        assert!(matches!(
            s.handle_events(KeyModifiers::NONE, KeyCode::Char('j')),
            EventResult::Consumed
        ));
    }

    #[test]
    /// UI-R-245 — PageDown/PageUp move by the visible height, Ctrl+D/Ctrl+U by half of
    /// it.
    fn ut_paging_moves_the_selection_by_the_visible_height_and_half_of_it() {
        let mut s = FileTreeStateBuilder::<FileStatus, NoBadge>::default()
            .paths(paths(&[
                ("a.rs", None),
                ("b.rs", None),
                ("c.rs", None),
                ("d.rs", None),
                ("e.rs", None),
            ]))
            .build()
            .unwrap();
        s.set_visible_height(2);
        s.handle_key(KeyModifiers::NONE, KeyCode::PageDown);
        assert_eq!(s.selected(), 2);
        s.handle_key(KeyModifiers::CONTROL, KeyCode::Char('d'));
        assert_eq!(s.selected(), 3);
        s.handle_key(KeyModifiers::NONE, KeyCode::PageUp);
        assert_eq!(s.selected(), 1);
        s.handle_key(KeyModifiers::CONTROL, KeyCode::Char('u'));
        assert_eq!(s.selected(), 0);
    }

    #[test]
    /// UI-R-245 — the viewport scrolls to keep the selected row visible.
    fn ut_scroll_offset_follows_the_selection_past_the_visible_height() {
        let mut s = FileTreeStateBuilder::<FileStatus, NoBadge>::default()
            .paths(paths(&[
                ("a.rs", None),
                ("b.rs", None),
                ("c.rs", None),
                ("d.rs", None),
            ]))
            .build()
            .unwrap();
        s.set_visible_height(2);
        assert_eq!(s.scroll_offset(), 0);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('j'));
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.selected(), 2);
        assert_eq!(s.scroll_offset(), 1);
    }

    #[test]
    /// UI-R-245 — `k` above the current viewport scrolls the offset back up to follow the
    /// selection, the mirror of the downward case above.
    fn ut_k_above_the_viewport_scrolls_the_offset_up() {
        let mut s = FileTreeStateBuilder::<FileStatus, NoBadge>::default()
            .paths(paths(&[
                ("a.rs", None),
                ("b.rs", None),
                ("c.rs", None),
                ("d.rs", None),
                ("e.rs", None),
            ]))
            .build()
            .unwrap();
        s.set_visible_height(2);
        for _ in 0..3 {
            s.handle_key(KeyModifiers::NONE, KeyCode::Char('j'));
        }
        assert_eq!(s.selected(), 3);
        assert_eq!(s.scroll_offset(), 2);
        for _ in 0..3 {
            s.handle_key(KeyModifiers::NONE, KeyCode::Char('k'));
        }
        assert_eq!(s.selected(), 0);
        assert_eq!(s.scroll_offset(), 0);
    }

    #[test]
    /// UI-R-245 — PageDown stops at the last visible row instead of moving past it.
    fn ut_page_down_stops_at_the_last_row() {
        let mut s = FileTreeStateBuilder::<FileStatus, NoBadge>::default()
            .paths(paths(&[
                ("a.rs", None),
                ("b.rs", None),
                ("c.rs", None),
                ("d.rs", None),
                ("e.rs", None),
            ]))
            .build()
            .unwrap();
        s.set_visible_height(2);
        s.handle_key(KeyModifiers::NONE, KeyCode::PageDown);
        assert_eq!(s.selected(), 2);
        s.handle_key(KeyModifiers::NONE, KeyCode::PageDown);
        assert_eq!(s.selected(), 4);
        s.handle_key(KeyModifiers::NONE, KeyCode::PageDown);
        assert_eq!(s.selected(), 4);
    }

    #[test]
    /// UI-R-234 — `set_paths` rebuilds the tree from a fresh path list and clamps a
    /// selection that no longer fits the new row count, resetting the viewport to keep it
    /// visible (UI-R-245).
    fn ut_set_paths_rebuilds_the_tree_and_clamps_the_selection() {
        let mut s = tree(&[("a.rs", None), ("b.rs", None), ("c.rs", None)]);
        s.set_visible_height(2);
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('G'));
        assert_eq!(s.selected(), 2);
        assert_eq!(s.scroll_offset(), 1);

        s.set_paths(&paths(&[("only.rs", None)]));
        let rows = s.visible_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "only.rs");
        assert_eq!(s.selected(), 0);
        assert_eq!(s.scroll_offset(), 0);
    }

    #[test]
    /// UI-E-103 — `Enter` on an empty tree reports consumed rather than unhandled.
    fn ut_enter_on_an_empty_tree_reports_consumed() {
        let mut s = tree(&[]);
        assert_eq!(
            s.handle_key(KeyModifiers::NONE, KeyCode::Enter),
            Some(FileTreeOutcome::Consumed)
        );
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Severity {
        Info,
    }

    impl FileTreeStatus for Severity {
        fn marker(&self) -> String {
            "!".to_string()
        }

        fn style(&self, _theme: &SyntaxTheme) -> Style {
            Style::default()
        }
    }

    #[test]
    /// UI-R-244 — `visible_rows()` hands back the caller's status type unchanged for files
    /// and `None` for directories.
    fn ut_visible_rows_carry_the_caller_status_type() {
        let s: FileTreeState<Severity> = FileTreeStateBuilder::default()
            .paths(vec![
                FileTreeEntry::new("a/b.rs").with_status(Severity::Info),
                FileTreeEntry::new("a/c.rs"),
            ])
            .build()
            .unwrap();
        let rows = s.visible_rows();
        assert_eq!(rows[0].status, None);
        assert_eq!(rows[1].status, Some(Severity::Info));
        assert_eq!(rows[2].status, None);
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Marker(&'static str);

    impl FileTreeBadge for Marker {
        fn text(&self) -> String {
            self.0.to_string()
        }

        fn style(&self) -> Option<Style> {
            None
        }
    }

    #[test]
    /// UI-R-314 — a badge supplied at construction is stored against the file's path and
    /// surfaced by `visible_rows()`.
    fn ut_badge_from_the_construction_input_is_stored_against_the_path() {
        let s: FileTreeState<FileStatus, Marker> = FileTreeStateBuilder::default()
            .paths(vec![
                FileTreeEntry::new("a.rs").with_badge(Marker("*")),
                FileTreeEntry::new("b.rs"),
            ])
            .build()
            .unwrap();
        let rows = s.visible_rows();
        assert_eq!(rows[0].badge, Some(Marker("*")));
        assert_eq!(rows[1].badge, None);
    }

    #[test]
    /// UI-R-317 — `set_badge` sets, replaces and (with `None`) clears a path's badge,
    /// leaving the selection and every directory's expansion unchanged.
    fn ut_set_badge_sets_replaces_and_clears_leaving_selection_and_expansion() {
        let mut s: FileTreeState<FileStatus, Marker> = FileTreeStateBuilder::default()
            .paths(paths(&[("a/b.rs", None), ("c.rs", None)]))
            .build()
            .unwrap();
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('j'));
        let selected_before = s.selected_path();
        let expanded_before: Vec<bool> = s.visible_rows().iter().map(|r| r.expanded).collect();

        s.set_badge("a/b.rs", Some(Marker("*")));
        assert_eq!(s.visible_rows()[1].badge, Some(Marker("*")));
        assert_eq!(s.selected_path(), selected_before);

        s.set_badge("a/b.rs", Some(Marker("!")));
        assert_eq!(s.visible_rows()[1].badge, Some(Marker("!")));

        s.set_badge("a/b.rs", None);
        assert_eq!(s.visible_rows()[1].badge, None);
        assert_eq!(s.selected_path(), selected_before);
        let expanded_after: Vec<bool> = s.visible_rows().iter().map(|r| r.expanded).collect();
        assert_eq!(expanded_before, expanded_after);
    }

    #[test]
    /// UI-E-148 — a badge set for a path matching no file node, or matching a directory,
    /// is stored but produces no badge on any visible row.
    fn ut_badge_for_an_unknown_or_directory_path_is_stored_and_never_drawn() {
        let mut s: FileTreeState<FileStatus, Marker> = FileTreeStateBuilder::default()
            .paths(paths(&[("a/b.rs", None)]))
            .build()
            .unwrap();
        s.set_badge("no/such/path", Some(Marker("*")));
        s.set_badge("a", Some(Marker("*")));
        assert_eq!(s.badges.get("no/such/path"), Some(&Marker("*")));
        assert_eq!(s.badges.get("a"), Some(&Marker("*")));
        for row in s.visible_rows() {
            assert_eq!(row.badge, None);
        }
    }

    #[test]
    /// UI-R-323 — the file tree's badge type defaults to the shipped no-badge type, which
    /// reports empty text and no style.
    fn ut_default_badge_type_yields_no_badge() {
        let badge = NoBadge;
        assert_eq!(badge.text(), "");
        assert_eq!(badge.style(), None);
    }
}
