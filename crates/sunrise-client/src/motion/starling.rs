//! Starling's part of motion.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl PlayerMotion {
    /// `ClientPacketListener.handleUpdateAttributes` for this player: the server's base value
    /// and modifiers (`(id, amount, operation)`) replace the instance's own.
    pub fn apply_snapshot(
        &mut self,
        attribute: &str,
        base: f64,
        modifiers: &[(&str, f64, i32)],
    ) -> Result<()> {
        let Some(key) = self.attributes.key(attribute) else {
            return Ok(());
        };
        let Some(handle) = self
            .attributes
            .get_instance(Some(key))
            .map_err(attribute_error)?
        else {
            return Ok(());
        };
        let modifiers = modifiers
            .iter()
            .map(|&(id, amount, operation)| {
                let operation = match operation {
                    1 => Operation::AddMultipliedBase,
                    2 => Operation::AddMultipliedTotal,
                    _ => Operation::AddValue,
                };
                Modifier::new(id, amount, operation)
            })
            .collect::<io::Result<Vec<_>>>()?;
        self.attributes
            .edit(&handle, |value, observer| {
                value.set_base_value(base, observer)?;
                value.remove_all(observer)?;
                for m in modifiers {
                    value.add_transient(m, observer)?;
                }
                Ok(())
            })
            .map_err(attribute_error)?;
        Ok(())
    }
}
