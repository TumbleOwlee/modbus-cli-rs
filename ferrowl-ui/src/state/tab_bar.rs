use crate::traits::ToLabel;

pub struct TabBarState<T: ToLabel> {
    pub titles: Vec<T>,
    pub active: usize,
    /// The first visible cell of the stacked titles, counted in character/padding
    /// cells along the layout direction across the whole tab list, not in tab indices.
    pub offset: usize,
}

impl<T: ToLabel> TabBarState<T> {
    /// UI-R-324 — a shared reference to the tab value at the active index, absent only when
    /// the tab list is empty.
    pub fn selected(&self) -> Option<&T> {
        self.titles.get(self.selected_index())
    }

    /// UI-R-325 — the active index normalized against the current tab list: a stored value
    /// that is out of range, however it got there, reads as `0`.
    pub fn selected_index(&self) -> usize {
        if self.active < self.titles.len() {
            self.active
        } else {
            0
        }
    }

    /// UI-R-326 — jump to `index`, clamped to the last tab.
    pub fn select_index(&mut self, index: usize) {
        self.active = index.min(self.titles.len().saturating_sub(1));
    }

    /// UI-R-327 — activate the next tab, wrapping from the last tab to the first.
    pub fn next(&mut self) {
        let len = self.titles.len();
        if len == 0 {
            self.active = 0;
            return;
        }
        let current = self.selected_index();
        self.active = if current + 1 >= len { 0 } else { current + 1 };
    }

    /// UI-R-328 — activate the previous tab, wrapping from the first tab to the last.
    pub fn previous(&mut self) {
        let len = self.titles.len();
        if len == 0 {
            self.active = 0;
            return;
        }
        let current = self.selected_index();
        self.active = if current == 0 { len - 1 } else { current - 1 };
    }

    /// UI-R-329 — replace the tab list, keeping the active index where it is when it is
    /// still valid for the new list and clamping it to the new list's last index otherwise.
    pub fn set_titles(&mut self, titles: Vec<T>) {
        let current = self.selected_index();
        self.titles = titles;
        self.active = current.min(self.titles.len().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> TabBarState<String> {
        TabBarState {
            titles: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            active: 0,
            offset: 0,
        }
    }

    /// UI-R-324 — `selected` reports the active tab's value, absent only for an empty list.
    #[test]
    fn ut_selected_reports_active_tab_value() {
        let mut s = state();
        s.active = 1;
        assert_eq!(s.selected(), Some(&"b".to_string()));

        let empty = TabBarState::<String> {
            titles: vec![],
            active: 0,
            offset: 0,
        };
        assert_eq!(empty.selected(), None);
    }

    /// UI-R-325 — every read of the active index normalizes it against the current tab list.
    #[test]
    fn ut_selected_index_normalizes_and_tracks_helpers() {
        let mut s = state();
        assert_eq!(s.selected_index(), 0);
        s.next();
        assert_eq!(s.selected_index(), 1);
        s.select_index(9);
        assert_eq!(s.selected_index(), 2);
        s.active = 9;
        assert_eq!(s.selected_index(), 0);
        s.active = 1;
        assert_eq!(s.selected_index() + 1, 2);
    }

    /// UI-R-326 — `select_index` clamps to the last tab.
    #[test]
    fn ut_select_index_clamps_to_last_tab() {
        let mut s = state();
        s.select_index(1);
        assert_eq!(s.selected_index(), 1);
        s.select_index(9);
        assert_eq!(s.selected_index(), 2);
    }

    /// UI-R-327 — `next` advances and wraps from the last tab to the first.
    #[test]
    fn ut_next_advances_and_wraps_at_end() {
        let mut s = state();
        s.next();
        assert_eq!(s.selected_index(), 1);
        s.next();
        assert_eq!(s.selected_index(), 2);
        s.next();
        assert_eq!(s.selected_index(), 0);
    }

    /// UI-R-328 — `previous` retreats and wraps from the first tab to the last.
    #[test]
    fn ut_previous_retreats_and_wraps_at_start() {
        let mut s = state();
        s.active = 2;
        s.previous();
        assert_eq!(s.selected_index(), 1);
        s.previous();
        assert_eq!(s.selected_index(), 0);
        s.previous();
        assert_eq!(s.selected_index(), 2);
    }

    /// UI-R-329 — `set_titles` keeps a still-valid active index and clamps otherwise.
    #[test]
    fn ut_set_titles_keeps_valid_active_and_clamps_otherwise() {
        let mut s = state();
        s.active = 2;
        s.set_titles(vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ]);
        assert_eq!(s.selected_index(), 2);

        let mut fresh = state();
        fresh.active = 2;
        fresh.set_titles(vec!["x".to_string(), "y".to_string()]);
        assert_eq!(fresh.selected_index(), 1);
    }

    /// UI-R-329, UI-R-325 — `set_titles` normalizes the outgoing active index before clamping
    /// it to the new list, so an already out-of-range value never survives as a valid-looking
    /// index into the new list.
    #[test]
    fn ut_set_titles_starts_from_the_normalized_index() {
        let mut s = state();
        s.active = 9;
        s.set_titles(vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
            "e".to_string(),
        ]);
        assert_eq!(s.selected_index(), 0);

        let mut valid = TabBarState::<String> {
            titles: vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
                "e".to_string(),
            ],
            active: 4,
            offset: 0,
        };
        valid.set_titles(vec!["x".to_string(), "y".to_string(), "z".to_string()]);
        assert_eq!(valid.selected_index(), 2);
    }

    /// UI-E-153 — `next`/`previous` on an empty tab list stay at index `0` and never panic.
    #[test]
    fn ut_next_and_previous_on_empty_list_stay_at_zero() {
        let mut a = TabBarState::<String> {
            titles: vec![],
            active: 2,
            offset: 0,
        };
        a.next();
        assert_eq!(a.active, 0);

        let mut b = TabBarState::<String> {
            titles: vec![],
            active: 2,
            offset: 0,
        };
        b.previous();
        assert_eq!(b.active, 0);
    }

    /// UI-E-154 — `next`/`previous` on a single-tab list wrap back onto that same tab.
    #[test]
    fn ut_next_and_previous_on_single_tab_stay_at_zero() {
        let mut s = TabBarState::<String> {
            titles: vec!["only".to_string()],
            active: 0,
            offset: 0,
        };
        s.next();
        assert_eq!(s.selected_index(), 0);
        assert_eq!(s.selected(), Some(&"only".to_string()));

        s.previous();
        assert_eq!(s.selected_index(), 0);
        assert_eq!(s.selected(), Some(&"only".to_string()));
    }

    /// UI-E-155 — `select_index` on an empty tab list yields `0`, never underflows.
    #[test]
    fn ut_select_index_on_empty_list_is_zero() {
        let mut s = TabBarState::<String> {
            titles: vec![],
            active: 2,
            offset: 0,
        };
        s.select_index(5);
        assert_eq!(s.active, 0);
    }

    /// UI-E-156 — `set_titles` with an empty list resets the active index to `0`.
    #[test]
    fn ut_set_titles_with_empty_list_resets_active_to_zero() {
        let mut s = state();
        s.active = 2;
        s.set_titles(vec![]);
        assert_eq!(s.selected_index(), 0);
        assert!(s.titles.is_empty());
    }

    /// UI-E-157 — an active index written out of range directly reads back as `0` and the
    /// first tab, never the written value and never absent for a non-empty list.
    #[test]
    fn ut_out_of_range_active_reads_as_first_tab() {
        let mut s = state();
        s.active = 9;
        assert_eq!(s.selected_index(), 0);
        assert_eq!(s.selected(), Some(&"a".to_string()));
    }

    /// UI-E-158 — a selection helper starting from an out-of-range active index steps from
    /// the normalized index `0`, never from the raw stored value.
    #[test]
    fn ut_out_of_range_active_steps_from_normalized_zero() {
        let mut next_s = state();
        next_s.active = 9;
        next_s.next();
        assert_eq!(next_s.selected_index(), 1);

        let mut prev_s = state();
        prev_s.active = 9;
        prev_s.previous();
        assert_eq!(prev_s.selected_index(), 2);

        let mut single = TabBarState::<String> {
            titles: vec!["only".to_string()],
            active: 5,
            offset: 0,
        };
        single.next();
        assert_eq!(single.selected_index(), 0);
    }
}
