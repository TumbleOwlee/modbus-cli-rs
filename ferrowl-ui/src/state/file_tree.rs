use derive_builder::Builder;

/// A file node's change status (UI-R-244): drawn as a leading marker and styled with the
/// syntax theme's added/removed/meta styles. Public because the caller sets it per path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Removed,
    Modified,
}

/// A node of the tree built from the paths [`FileTreeState`] is constructed with. Crate-private
/// like `DiffRow` in `diff_view.rs`: no `api-contract.md` row exposes the tree itself,
/// only the path list going in and the selected path coming out.
#[derive(Debug, Clone)]
pub(crate) enum TreeNode {
    Dir {
        name: String,
        children: Vec<TreeNode>,
        expanded: bool,
    },
    File {
        name: String,
        status: Option<FileStatus>,
    },
}

/// One row of `FileTreeState::visible_rows()`'s depth-first walk. Crate-private: the
/// widget that renders this state is its only other caller, and no `api-contract.md` row
/// exposes the row list.
#[derive(Debug, Clone)]
pub(crate) struct VisibleRow {
    // Read by the widget that renders this state, not by anything in this module outside
    // tests.
    #[allow(dead_code)]
    pub(crate) depth: usize,
    pub(crate) path: String,
    pub(crate) is_dir: bool,
    // Read by the widget that renders this state, not by anything in this module outside
    // tests.
    #[allow(dead_code)]
    pub(crate) expanded: bool,
    #[allow(dead_code)]
    pub(crate) name: String,
    #[allow(dead_code)]
    pub(crate) status: Option<FileStatus>,
}

/// Splits each path on `/`, creating the directory nodes its components imply and hanging
/// the file under the last one (UI-R-234); a path with no `/` becomes a file node directly
/// under the root (UI-E-104). Every directory created is `expanded: true` (UI-R-235).
fn build_tree(paths: &[(String, Option<FileStatus>)]) -> Vec<TreeNode> {
    let mut root: Vec<TreeNode> = Vec::new();
    for (path, status) in paths {
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
            status: *status,
        });
    }
    root
}

fn push_visible(nodes: &[TreeNode], depth: usize, prefix: &str, out: &mut Vec<VisibleRow>) {
    let mut dirs: Vec<&TreeNode> = nodes
        .iter()
        .filter(|n| matches!(n, TreeNode::Dir { .. }))
        .collect();
    dirs.sort_by_key(|n| match n {
        TreeNode::Dir { name, .. } => name.clone(),
        TreeNode::File { .. } => unreachable!(),
    });
    let mut files: Vec<&TreeNode> = nodes
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
                });
                if *expanded {
                    push_visible(children, depth + 1, &path, out);
                }
            }
            TreeNode::File { name, status } => {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                out.push(VisibleRow {
                    depth,
                    path,
                    is_dir: false,
                    expanded: false,
                    name: name.clone(),
                    status: *status,
                });
            }
        }
    }
}

fn walk_mut(nodes: &mut [TreeNode], f: &mut impl FnMut(&mut bool)) {
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
pub struct FileTreeState {
    #[builder(setter(custom), default = "Vec::new()")]
    root: Vec<TreeNode>,
    #[builder(setter(skip), default = "0")]
    selected: usize,
    // Written by set_paths, read once key handling added on top of this file uses it to
    // keep the selection visible.
    #[allow(dead_code)]
    #[builder(setter(skip), default = "0")]
    scroll_offset: usize,
    // Set by the builder/widget, read once key handling added on top of this file uses it
    // for paging.
    #[allow(dead_code)]
    #[builder(default = "1")]
    visible_height: usize,
}

impl FileTreeStateBuilder {
    /// UI-R-234 — path plus optional status, routed through `build_tree`.
    pub fn paths(&mut self, paths: Vec<(String, Option<FileStatus>)>) -> &mut Self {
        self.root = Some(build_tree(&paths));
        self
    }
}

impl Default for FileTreeState {
    fn default() -> Self {
        FileTreeStateBuilder::default()
            .build()
            .expect("FileTreeStateBuilder fields all default")
    }
}

impl FileTreeState {
    /// UI-R-234 — rebuilds the tree from a fresh path list, routed through `build_tree`
    /// like the builder's `paths` setter; the selection is clamped to the new row count.
    pub fn set_paths(&mut self, paths: &[(String, Option<FileStatus>)]) {
        self.root = build_tree(paths);
        let rows = self.visible_rows();
        self.selected = self.selected.min(rows.len().saturating_sub(1));
        self.scroll_offset = 0;
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
    pub(crate) fn visible_rows(&self) -> Vec<VisibleRow> {
        let mut out = Vec::new();
        push_visible(&self.root, 0, "", &mut out);
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

    // Used directly by tests here and by key handling added on top of this file.
    #[allow(dead_code)]
    fn set_expanded(&mut self, path: &str, expanded: bool) {
        fn go(nodes: &mut [TreeNode], prefix: &str, path: &str, expanded: bool) -> bool {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[(&str, Option<FileStatus>)]) -> Vec<(String, Option<FileStatus>)> {
        list.iter().map(|(p, s)| (p.to_string(), *s)).collect()
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
        s.selected = 1;
        assert_eq!(s.selected_path().as_deref(), Some("a/b.rs"));
        assert_eq!(s.selected_is_dir(), Some(false));
    }

    #[test]
    /// UI-R-234 — `set_paths` rebuilds the tree from a fresh path list and clamps a
    /// selection that no longer fits the new row count.
    fn ut_set_paths_rebuilds_the_tree_and_clamps_the_selection() {
        let mut s = tree(&[("a.rs", None), ("b.rs", None), ("c.rs", None)]);
        s.selected = 2;

        s.set_paths(&paths(&[("only.rs", None)]));
        let rows = s.visible_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "only.rs");
        assert_eq!(s.selected, 0);
    }
}
