pub mod item {
    pub mod menu {
        pub struct MenuButtonContext {
            pub player: u128,
            pub player_entity_id: i32,
            pub pos: Option<[i32; 3]>,
            pub id: i32,
        }
        pub struct AnvilCraftInputs<'a>(std::marker::PhantomData<&'a ()>);
        pub struct AnvilUpdateInputs<'a>(std::marker::PhantomData<&'a ()>);
        #[derive(Clone)]
        pub struct AnvilUpdateEdits;
        pub struct GrindstonePlaceInputs<'a>(std::marker::PhantomData<&'a ()>);
        #[derive(Clone)]
        pub struct GrindstonePlaceEdits;
        pub struct GrindstoneTakeInputs<'a>(std::marker::PhantomData<&'a ()>);
        #[derive(Clone)]
        pub struct GrindstoneTakeEdits;
        pub struct StackedOnOther<'a>(pub std::marker::PhantomData<&'a mut ()>);
        impl StackedOnOther<'_> {
            pub fn checkpoint(&self) -> usize {
                0
            }
            pub fn rollback(&mut self, _: usize) {}
        }
    }
    pub mod stations {
        pub struct EnchantmentRow<'a>(std::marker::PhantomData<&'a ()>);
    }
}
