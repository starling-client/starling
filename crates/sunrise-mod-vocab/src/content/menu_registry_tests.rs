//! Menu iteration belongs to the Parts installation, not the process.
use super::*;

#[test]
fn another_installation_cannot_replace_a_menus_iteration_order() {
    let (published, wait_published) = std::sync::mpsc::channel();
    let (replaced, wait_replaced) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let owner = scope.spawn(move || {
            crate::parts::install_input(Default::default());
            resolve_menus(["geometry:menu"]).unwrap();
            let key = menu_keys()[0];
            published.send(()).unwrap();
            wait_replaced.recv().unwrap();
            assert_eq!(menu_keys(), [key]);
            assert_eq!(menu_key_id(key).as_deref(), Some("geometry:menu"));
            let entry = crate::RegistryEntry::new(
                crate::Identifier::parse("minecraft:menu").unwrap(),
                crate::Identifier::parse("geometry:menu").unwrap(),
            );
            assert_eq!(
                crate::wire::wire_types::registry_entry_key_slot(&entry),
                Some(key.0)
            );
            // Re-resolving one installation may change iteration order, but a
            // retained key must keep naming the same menu.
            resolve_menus(["bound:menu", "geometry:menu"]).unwrap();
            assert_eq!(menu_keys()[1], key);
            assert_eq!(menu_key_id(key).as_deref(), Some("geometry:menu"));
        });
        scope
            .spawn(move || {
                wait_published.recv().unwrap();
                crate::parts::install_input(Default::default());
                resolve_menus(["bound:menu"]).unwrap();
                replaced.send(()).unwrap();
            })
            .join()
            .unwrap();
        owner.join().unwrap();
    });
}
