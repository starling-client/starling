pub struct ItemEntityServerScope<'a>(std::marker::PhantomData<&'a mut ()>);
impl ItemEntityServerScope<'_> {
    pub fn route_item(&self) -> &str {
        "minecraft:stick"
    }
}
