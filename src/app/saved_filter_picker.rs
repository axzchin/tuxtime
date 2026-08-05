//! Saved-filter picker state: restore point and cursor index for the `ff` picker.

#[derive(Debug, Default)]
pub struct SavedFilterPicker {
    /// The search string that was active when the `ff` picker opened, so
    /// cancelling (`Esc`) restores it instead of leaving the previewed
    /// filter applied. `None` outside `Mode::PickSavedFilter`.
    pub restore: Option<String>,
    /// Index into `saved_filters` of the row the `ff` picker currently
    /// previews.
    pub idx: usize,
}
