use crate::traits::ToLabel;

pub struct TabBarState<T: ToLabel> {
    pub titles: Vec<T>,
    pub active: usize,
    /// The first visible cell of the stacked titles, counted in character/padding
    /// cells along the layout direction across the whole tab list, not in tab indices.
    pub offset: usize,
}
