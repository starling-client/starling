/// A command completion replacing a byte range of the input.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Suggestion {
    /// Start (inclusive) and end (exclusive) byte offsets.
    pub range: (usize, usize),
    /// Replacement text.
    pub text: String,
    /// Optional explanation of the completion.
    pub tooltip: Option<String>,
    /// Numeric sort value for integer completions.
    pub int_value: Option<i32>,
}
impl Suggestion {
    /// Construct a completion without a tooltip or numeric sort value.
    pub fn new(range: (usize, usize), text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
            tooltip: None,
            int_value: None,
        }
    }

    /// Attach an explanation tooltip shown beside this completion.
    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Override the start (inclusive) and end (exclusive) byte offsets replaced by this completion.
    pub fn with_range(mut self, range: (usize, usize)) -> Self {
        self.range = range;
        self
    }

    /// Set the numeric sort value used when sorting integer completions.
    pub fn with_int_value(mut self, value: i32) -> Self {
        self.int_value = Some(value);
        self
    }
}
