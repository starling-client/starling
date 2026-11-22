#[derive(Clone, Copy)]
pub struct BlockEntityLevelReadScope<'a>(std::marker::PhantomData<&'a ()>);
