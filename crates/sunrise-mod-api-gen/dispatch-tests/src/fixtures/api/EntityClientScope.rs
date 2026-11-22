pub struct EntityClientScope<'a>(std::marker::PhantomData<&'a mut ()>);
impl EntityClientScope<'_> {
    pub fn route_kind(&self) -> Option<&str> {
        None
    }
}
