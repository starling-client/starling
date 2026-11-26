pub trait Notes {
    #[notes = r#"## Recipe book view

`player.visit_discovered_recipes(host, &mut visit)` visits every recipe the player's book knows;
`has_discovered_recipe` asks about one at a time. The visitor borrows each qualified recipe id as
`&str`; copy it inside the callback if you need to keep it. Return `ControlFlow::Continue(())` to
keep visiting or `ControlFlow::Break(())` to stop. An empty book completes without a callback;
an unavailable or stale player gives `EntityReadError` instead. The visit reads the resting
guest's book outside their turn and their acting book during it. Use `award_recipes` and
`reset_recipes` to change the book."#]
    fn player_stats_note0();
}
