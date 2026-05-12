#[derive(Debug, Clone)]
pub enum ProgressUpdate {
    Step { message: String, current: usize, total: usize },
    Indeterminate { message: String },
    Done { summary: String },
    Error { message: String, action_label: Option<String> },
    /// Last completed folder status line (shown as subtitle in the GUI).
    StatusLine { text: String },
    /// Close the progress window immediately without showing a summary.
    AutoClose,
}
