//! Links `sunrise_threads::test_game_dir`, which starts the test process in a game folder of its own.
#[used]
static KEEP: fn() = sunrise_threads::test_game_dir::keep;
