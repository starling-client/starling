#[derive(Clone, Copy)]
pub struct LivingView<'a>(pub std::marker::PhantomData<&'a ()>);
