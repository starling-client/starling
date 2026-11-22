#[derive(Clone, Copy)]
pub struct BlockEntityClientReadScope<'a>(std::marker::PhantomData<&'a ()>);
impl BlockEntityClientReadScope<'_> {
    pub fn route_kind(&self) -> Option<&str> {
        None
    }
}
