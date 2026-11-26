mod stubs {
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub(super) enum RecipeBookPlacement {
        #[default]
        Nothing,
        Ghost,
        Grid {
            width: u16,
            height: u16,
            slots: Vec<u16>,
            clear: Vec<u16>,
        },
    }
}

#[test]
fn panicking_recipe_book_place_does_not_publish_candidate_output() {
    install(Vec::<ModEntry>::new().into());
    struct PanickingMenu;
    impl MenuPart for PanickingMenu {
        fn menu_subscribed(&self) -> &'static [Option<HookId>] {
            &[Some(HookId::RecipeBookPlace)]
        }
        fn recipe_book_place(&self, _: &mut dyn ServerHost, event: &mut RecipeBookPlaceEvent<'_>) {
            *event.placement = RecipeBookPlacement::Grid {
                width: 1,
                height: 1,
                slots: vec![0],
                clear: vec![0],
            };
            panic!("menu callback failure");
        }
    }
    install_parts(Parts {
        menus: vec![leak(PanickingMenu)],
        ..Parts::default()
    });
    let mut host = TestHost;
    let mut placement = RecipeBookPlacement::Nothing;
    dispatch_recipe_book_place(
        &mut host,
        calls::RecipeBookPlaceCall {
            menu: "test:menu",
            recipe: "test:recipe",
            use_max_items: false,
            creative: false,
            placement: &mut placement,
        },
    );
    assert_eq!(placement, RecipeBookPlacement::Nothing);
    install_parts(Parts::default());
}
