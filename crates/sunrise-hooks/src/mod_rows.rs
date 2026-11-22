//! Mods screen rows (docs/layers.md, "Platform registries"): a crate lists something the game
//! ships that is not an installed mod. The screen shows Minecraft, every registered row and the
//! installed mods, sorted as it sorts its list; with none registered it adds no row.
use crate::TooLate;
use crate::registry::Registry;

/// One row. `authors` and `description` are translation keys; the others are proper names,
/// versions and addresses, shown as written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub authors: &'static str,
    pub license: &'static str,
    pub license_url: Option<&'static str>,
    pub homepage: Option<&'static str>,
    pub issues: Option<&'static str>,
    pub description: &'static str,
}

static ROWS: Registry<Row> = Registry::new();

/// Lists `row` on the Mods screen; rows are kept in registration order, and the screen sorts what
/// it shows. Before the game starts, as [`crate::register`].
pub fn register(row: &'static Row) -> Result<(), TooLate> {
    ROWS.register(row)
}

/// The engine's side. Only the engine calls it.
pub mod engine {
    use super::*;

    /// Every registered row in registration order, sealing the list on the first call.
    pub fn all() -> &'static [&'static Row] {
        ROWS.all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_row_is_listed_in_order_and_late_ones_are_refused() {
        static FIRST: Row = row("first");
        static SECOND: Row = row("second");
        register(&FIRST).unwrap();
        register(&SECOND).unwrap();
        let ids: Vec<_> = engine::all().iter().map(|r| r.id).collect();
        assert_eq!(ids, ["first", "second"]);
        assert!(register(&FIRST).is_err());
    }

    const fn row(id: &'static str) -> Row {
        Row {
            id,
            name: id,
            version: "1",
            authors: "test.authors",
            license: "MIT",
            license_url: None,
            homepage: None,
            issues: None,
            description: "test.description",
        }
    }
}
