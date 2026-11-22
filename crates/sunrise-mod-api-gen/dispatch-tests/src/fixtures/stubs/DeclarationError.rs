#[derive(Debug)]
pub(super) enum DeclarationError {
    Rejected { id: String, reason: String },
}
