use super::*;

impl DamageTypesRegistrar for PartRegistrar {
    fn damage_type_key(&mut self, id: &str) -> crate::DamageTypeKey {
        match self
            .registry_keys
            .borrow_mut()
            .damage_types
            .request(self.mod_id, id)
        {
            Ok(slot) => mod_vocab::__private::parts_installation::damage_type_key(slot),
            Err(error) => {
                self.refused.push(error);
                mod_vocab::__private::parts_installation::damage_type_key(u32::MAX)
            }
        }
    }
}
