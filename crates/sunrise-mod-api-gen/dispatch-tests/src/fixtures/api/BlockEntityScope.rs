pub struct BlockEntityScope<'a>(pub Option<&'a mut super::super::ModBlockEntity>);

impl BlockEntityScope<'_> {
    pub fn with_view<R>(
        &mut self,
        f: impl FnOnce(&mut super::super::ModBlockEntity) -> R,
    ) -> Option<R> {
        self.0.as_deref_mut().map(f)
    }
    pub fn reborrow(&mut self) -> BlockEntityScope<'_> {
        BlockEntityScope(self.0.as_deref_mut())
    }
    pub fn transaction<T, E>(
        &mut self,
        f: impl FnOnce(&mut BlockEntityScope<'_>, &()) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut scope = self.reborrow();
        f(&mut scope, &())
    }
}
