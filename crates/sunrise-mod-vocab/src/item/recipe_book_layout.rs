//! Menu-owned destination roles, not recipe ingredient semantics.
/// Recipe destinations at physical container indices, never indices into the
/// appended player inventory. The menu owns these roles; the registered screen
/// factory supplies client geometry separately. Local slot moves and hides do
/// not change the destination of a recipe.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the four destination maps cover the five vanilla displays, and `Mod` hands the rest to the mod
pub enum RecipeBookLayout {
    Crafting {
        width: u16,
        height: u16,
        slots: Vec<u16>,
        result: u16,
    },
    Furnace {
        input: u16,
        fuel: u16,
        result: u16,
    },
    Stonecutter {
        input: u16,
        result: u16,
    },
    Smithing {
        template: u16,
        base: u16,
        addition: u16,
        result: u16,
    },
    /// The menu's placement is answered by the mod: the `recipe_book_place` hook of its
    /// `MenuPart` writes a [`RecipeBookPlacement`] (`Nothing`, `Ghost`, or `Grid` to invoke
    /// vanilla's placer on the active menu's physical slots). The roles are the mod's own, so
    /// none is declared or checked here, and the game maps no display to a slot.
    Mod,
}

/// How a `RecipeBookLayout::Mod` menu's `MenuPart::recipe_book_place` answers a recipe click.
// no wire: answer from a local menu callback, not a packet
// exhaustive: the three outcomes of a menu placement callback: custom/no-op, ghost, or engine grid placement
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RecipeBookPlacement {
    /// Leave placement to the part's own normal `Host` operations, or do nothing.
    #[default]
    Nothing,
    /// The manual menu says ingredients are missing; while the same opening remains current, the
    /// server sends the clicked display to the client to show the recipe's ghosts.
    Ghost,
    /// Ask the engine's canonical placer (`ServerPlaceRecipe.placeRecipe`) to place the clicked
    /// recipe into these physical container slots on the active menu and its live storage, using
    /// the click's current Shift (`use_max_items`) and creative flags and sending ghosts only when
    /// ingredients are missing.
    Grid {
        /// Grid width in cells.
        width: u16,
        /// Grid height in cells.
        height: u16,
        /// Physical menu container slot index for each cell of the grid, row by row.
        slots: Vec<u16>,
        /// Physical menu container slots emptied back into the player's inventory first (the
        /// grid's own slots are cleared too).
        clear: Vec<u16>,
    },
}

impl RecipeBookLayout {
    /// Engine integration: diagnostics retain the actual declaring mod identity.
    // hidden: engine diagnostics need registration ownership, not a mod-facing lookup service.
    #[doc(hidden)]
    pub fn declaring_mod(menu: &str) -> Option<&'static str> {
        crate::parts::installed()?
            .menu_recipe_books
            .get(menu)
            .map(|binding| binding.owner)
    }
    /// Menus with declarations, for content-load validation, never per tick.
    pub fn registered_menus() -> Vec<String> {
        let mut menus = crate::parts::installed()
            .map(|r| r.menu_recipe_books.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        menus.sort();
        menus
    }
    /// Frozen declaration for a menu, as registered at startup. No declaration
    /// means that the menu has no custom-book placement contract.
    pub fn registered(menu: &str) -> Option<(crate::RecipeBookTypeKey, String, Self)> {
        crate::parts::installed()?
            .menu_recipe_books
            .get(menu)
            .map(|binding| {
                (
                    binding.book.clone(),
                    binding.recipe_type.clone(),
                    binding.layout.clone(),
                )
            })
    }

    /// Check role indices against the content menu's container slot count.
    pub fn validate(&self, slot_count: u16) -> Result<(), String> {
        if slot_count == 0 {
            return Err("container slot count must be at least 1".into());
        }
        if let Self::Crafting {
            width,
            height,
            slots,
            ..
        } = self
        {
            let cells = u32::from(*width) * u32::from(*height);
            if *width == 0
                || *height == 0
                || cells > u32::from(slot_count)
                || cells as usize != slots.len()
            {
                return Err("grid dimensions must match its slot list and fit the menu".into());
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        self.check_roles(|slot, role| {
            if slot >= slot_count {
                return Err(format!("{role} slot {slot} is outside the menu"));
            }
            if !seen.insert(slot) {
                return Err(format!("{role} slot {slot} overlaps another role"));
            }
            Ok(())
        })
    }

    fn check_roles(
        &self,
        mut check: impl FnMut(u16, &'static str) -> Result<(), String>,
    ) -> Result<(), String> {
        match self {
            Self::Crafting { slots, result, .. } => {
                for &slot in slots {
                    check(slot, "grid")?;
                }
                check(*result, "result")
            }
            Self::Furnace {
                input,
                fuel,
                result,
            } => {
                check(*input, "input")?;
                check(*fuel, "fuel")?;
                check(*result, "result")
            }
            Self::Stonecutter { input, result } => {
                check(*input, "input")?;
                check(*result, "result")
            }
            Self::Smithing {
                template,
                base,
                addition,
                result,
            } => {
                check(*template, "template")?;
                check(*base, "base")?;
                check(*addition, "addition")?;
                check(*result, "result")
            }
            Self::Mod => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RecipeBookLayout;

    #[test]
    fn validate_accepts_more_than_256_slots_and_rejects_bad_indices() {
        let big = RecipeBookLayout::Crafting {
            width: 3,
            height: 3,
            slots: (290..299).collect(),
            result: 299,
        };
        assert_eq!(big.validate(300), Ok(()));
        assert!(big.validate(299).is_err());
        assert!(big.validate(0).is_err());

        let overlap = RecipeBookLayout::Furnace {
            input: 270,
            fuel: 271,
            result: 270,
        };
        assert!(overlap.validate(300).is_err());
    }
}
