#[derive(Clone, Copy)]
pub(super) struct CommandTreeFilter<'a>(std::marker::PhantomData<&'a ()>);
