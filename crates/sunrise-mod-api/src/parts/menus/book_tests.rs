use super::*;
fn roles() -> crate::RecipeBookLayout {
    crate::RecipeBookLayout::Furnace {
        input: 0,
        fuel: 1,
        result: 2,
    }
}
#[test]
fn registration_refuses_unknown_books_duplicate_menus_and_overlapping_roles() {
    let mut registrar = PartRegistrar::new("demo");
    let book = registrar.recipe_book_type("demo:book").unwrap();
    registrar
        .register_menu_screen(
            "demo:pot",
            Arc::new(|_: &dyn ClientHost, _: crate::MenuScreenOpen<'_>| None),
        )
        .unwrap();
    assert!(
        registrar
            .menu_recipe_book("demo:pot", &book, "demo:cooking", roles())
            .is_ok()
    );
    assert!(
        registrar
            .menu_recipe_book("demo:pot", &book, "demo:cooking", roles())
            .is_err()
    );
    assert!(
        registrar
            .menu_recipe_book("other:pot", &book, "demo:cooking", roles())
            .is_err()
    );
    assert!(
        registrar
            .menu_recipe_book(
                "demo:bad",
                &book,
                "demo:cooking",
                crate::RecipeBookLayout::Stonecutter {
                    input: 0,
                    result: 0
                },
            )
            .is_err()
    );
    let unknown = mod_vocab::__private::parts_installation::recipe_book_type("demo:unknown".into());
    assert!(
        registrar
            .menu_recipe_book("demo:unknown", &unknown, "demo:cooking", roles())
            .is_err()
    );
    assert_eq!(registrar.menu_recipe_books.len(), 1);
    assert_eq!(registrar.refused.len(), 4);
    let binding = &registrar.menu_recipe_books["demo:pot"];
    assert_eq!(binding.owner, "demo");
    assert_eq!(binding.book, book);
    assert_eq!(binding.recipe_type, "demo:cooking");
    assert_eq!(binding.layout, roles());
}

#[test]
fn menu_recipe_book_accepts_served_content_namespace_only_for_its_host() {
    let packages = vec![
        crate::PackageInfo::new(
            "horizon_java".into(),
            "1".into(),
            crate::PackageKind::Code,
            crate::Side::Both,
            vec!["horizon_java".into()],
            vec![],
        ),
        crate::PackageInfo::new(
            "third".into(),
            "1".into(),
            crate::PackageKind::Code,
            crate::Side::Both,
            vec!["third".into()],
            vec![],
        ),
        crate::PackageInfo::new(
            "runes".into(),
            "1".into(),
            crate::PackageKind::Content,
            crate::Side::Both,
            vec!["runes".into()],
            vec![],
        )
        .with_served_by(Some("horizon_java".into())),
        crate::PackageInfo::new(
            "plain".into(),
            "1".into(),
            crate::PackageKind::Content,
            crate::Side::Both,
            vec!["plain".into()],
            vec![],
        ),
    ];
    crate::load_state::with_test_package_inventory(packages, || {
        let mut host = PartRegistrar::new("horizon_java");
        let book = host.recipe_book_type("runes:book").unwrap();
        assert!(
            host.menu_recipe_book("runes:altar", &book, "runes:crafting", roles())
                .is_ok()
        );
        assert!(
            host.menu_recipe_book("plain:table", &book, "runes:crafting", roles())
                .is_err()
        );
        assert!(
            host.menu_recipe_book("minecraft:crafting", &book, "runes:crafting", roles())
                .is_err()
        );

        let mut third = PartRegistrar::new("third");
        let third_book = third.recipe_book_type("third:book").unwrap();
        assert!(
            third
                .menu_recipe_book("runes:altar", &third_book, "runes:crafting", roles())
                .is_err()
        );
    });
}
