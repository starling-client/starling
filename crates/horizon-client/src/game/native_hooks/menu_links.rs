//! Engine fallback while public API boundary forms remain held.
impl super::Game<'_> {
    pub(in crate::game) fn native_menu_links(
        &mut self,
        _player: u128,
        _menu: &str,
        _pos: [i32; 3],
        _links: &mut Vec<[i32; 3]>,
    ) {
    }
}
