use super::*;

fn poses(cells: &[(i32, i32)]) -> Vec<[f32; 6]> {
    let overlay_view = OverlayView {
        x: 0,
        y: 0,
        recipes: vec![OverlayRecipe {
            craftable: true,
            cells: cells
                .iter()
                .map(|&(x, y)| (x, y, vec![ItemStack::new("minecraft:stone", 1)]))
                .collect(),
        }],
    };
    let search = Search::default();
    let screen = Screen {
        book: BookType::Crafting,
        origin: (0, 37),
        mouse: (-1, -1),
        index: 0,
        filtering: false,
        filter_look: None,
        search: &search,
        tabs: &[],
        buttons: &[],
        page: 0,
        pages: 1,
        overlay: None,
        ghost: None,
        translate: &str::to_owned,
        text_width: &|text| text.len() as i32,
    };
    let mut poses = Vec::new();
    overlay(&screen, &overlay_view, &mut |call, pose| {
        if matches!(call, Call::Item { .. }) {
            poses.push(pose);
        }
    });
    poses
}

fn assert_inner_bounds(poses: &[[f32; 6]]) {
    // First tile is (4, 5), its 20-pixel inner area starts at (6, 7).
    for pose in poses {
        assert!(pose[4] >= 6.0 - 0.00001 && pose[5] >= 7.0 - 0.00001);
        assert!(pose[4] + 16.0 * pose[0] <= 26.0 + 0.00001);
        assert!(pose[5] + 16.0 * pose[3] <= 27.0 + 0.00001);
        assert_eq!(pose[0], pose[3]);
    }
}

#[test]
fn four_wide_recipe_fits_inner_tile() {
    let poses = poses(&[(0, 0), (1, 0), (2, 0), (3, 0)]);
    assert_inner_bounds(&poses);
    assert!((poses[0][0] - 0.2777778).abs() < 0.00001);
    assert!((poses[3][4] - 21.555555).abs() < 0.00001);
}

#[test]
fn twelve_ingredient_recipe_fits_inner_tile() {
    let cells: Vec<_> = (0..12).map(|i| (i % 3, i / 3)).collect();
    let poses = poses(&cells);
    assert_eq!(poses.len(), 12);
    assert_inner_bounds(&poses);
    assert!((poses[11][5] - 22.555555).abs() < 0.00001);
}

#[test]
fn vanilla_three_by_three_poses_are_unchanged() {
    let cells: Vec<_> = (0..9).map(|i| (i % 3, i / 3)).collect();
    for ((x, y), pose) in cells.iter().zip(poses(&cells)) {
        assert_eq!(
            pose,
            [
                0.375,
                0.0,
                0.0,
                0.375,
                6.0 + *x as f32 * 7.0,
                7.0 + *y as f32 * 7.0
            ]
        );
    }
}

#[test]
fn extreme_cell_coordinates_do_not_overflow() {
    assert_inner_bounds(&poses(&[(0, 0), (i32::MAX, i32::MAX)]));
}

#[test]
fn explicit_origin_moves_widgets_but_not_absolute_overlay_or_ghosts() {
    let search = Search::default();
    let tabs = [TabView {
        icons: ("minecraft:stone".into(), None),
        selected: true,
        anim: 0.0,
    }];
    let mut screen = Screen {
        book: BookType::Crafting,
        origin: (110, 37),
        mouse: (221, 50),
        index: 0,
        filtering: false,
        filter_look: None,
        search: &search,
        tabs: &tabs,
        buttons: &[],
        page: 0,
        pages: 1,
        overlay: None,
        ghost: None,
        translate: &str::to_owned,
        text_width: &|s| s.len() as i32,
    };
    let calls = draw(&screen);
    assert!(matches!(calls[0].call, Call::Blit { x: 110, y: 37, .. }));
    assert!(calls.iter().any(|c| matches!(&c.call, Call::Sprite { id, x: 78, y: 40, .. } if id == "recipe_book/tab_selected")));
    assert!(calls.iter().any(|c| matches!(&c.call, Call::Sprite { id, x: 220, y: 49, .. } if id == "recipe_book/filter_disabled_highlighted")));
    let overlay = OverlayView {
        x: 300,
        y: 80,
        recipes: vec![OverlayRecipe {
            craftable: true,
            cells: vec![],
        }],
    };
    screen.overlay = Some(&overlay);
    let ghosts = [GhostView {
        x: 8,
        y: 9,
        items: vec![ItemStack::new("minecraft:stone", 1)],
        result: false,
    }];
    screen.ghost = Some((400, 100, false, &ghosts));
    let calls = draw(&screen);
    assert!(
        calls
            .iter()
            .any(|c| matches!(&c.call, Call::Sprite { x: 300, y: 80, .. }))
    );
    assert!(
        calls
            .iter()
            .any(|c| matches!(&c.call, Call::Item { x: 8, y: 9, .. })
                && c.pose == [1.0, 0.0, 0.0, 1.0, 400.0, 100.0])
    );
}

#[test]
fn a_book_draws_its_own_filter_look_and_two_icon_tabs() {
    let look = mod_api::RecipeBookFilterLook {
        enabled: "demo:book/on".into(),
        disabled: "demo:book/off".into(),
        enabled_highlighted: "demo:book/on_hover".into(),
        disabled_highlighted: "demo:book/off_hover".into(),
        tooltip: "demo.book.cookable".into(),
    };
    let search = Search::default();
    let tabs = [TabView {
        icons: ("demo:dumpling".into(), Some("demo:sauce".into())),
        selected: false,
        anim: 0.0,
    }];
    let mut screen = Screen {
        book: BookType::Crafting,
        origin: (110, 37),
        mouse: (221, 50),
        index: 0,
        filtering: true,
        filter_look: Some(&look),
        search: &search,
        tabs: &tabs,
        buttons: &[],
        page: 0,
        pages: 1,
        overlay: None,
        ghost: None,
        translate: &str::to_owned,
        text_width: &|s| s.len() as i32,
    };
    let sprites = |screen: &Screen| -> Vec<String> {
        draw(screen)
            .iter()
            .filter_map(|c| match &c.call {
                Call::Sprite {
                    id, x: 220, y: 49, ..
                } => Some(id.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(sprites(&screen), ["demo:book/on_hover"]);
    let tooltip = |screen: &Screen| -> Vec<Vec<Run>> {
        draw(screen)
            .into_iter()
            .find_map(|c| match c.call {
                Call::Tooltip(lines) => Some(lines),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(
        tooltip(&screen),
        [[Run::plain("demo.book.cookable".to_owned())]]
    );
    screen.filtering = false;
    assert_eq!(sprites(&screen), ["demo:book/off_hover"]);
    assert_eq!(
        tooltip(&screen),
        [[Run::plain("gui.recipebook.toggleRecipes.all".to_owned())]]
    );
    screen.mouse = (0, 0);
    assert_eq!(sprites(&screen), ["demo:book/off"]);
    screen.filtering = true;
    assert_eq!(sprites(&screen), ["demo:book/on"]);
    // The tab sits at x = 110 - 30 and y = 37 + 3; its icons are 3 and 14 pixels in, 5 down.
    let icons: Vec<_> = draw(&screen)
        .iter()
        .filter_map(|c| match &c.call {
            Call::Item { stack, x, y, .. } => Some((stack.item.to_string(), *x, *y)),
            _ => None,
        })
        .collect();
    assert_eq!(
        icons,
        [
            ("demo:dumpling".to_owned(), 83, 45),
            ("demo:sauce".to_owned(), 94, 45)
        ]
    );
}
