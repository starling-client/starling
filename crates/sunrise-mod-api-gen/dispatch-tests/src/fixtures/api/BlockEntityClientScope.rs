pub struct BlockEntityClientScope<'a>(std::marker::PhantomData<&'a mut ()>);
impl BlockEntityClientScope<'_> {
    pub fn route_kind(&self) -> Option<&str> {
        None
    }
}
