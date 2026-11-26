#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountProfileProperty {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}
