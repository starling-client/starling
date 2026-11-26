mod stubs {
    pub(super) struct UnspawnedMobView<'a>(pub std::marker::PhantomData<&'a ()>);
    pub(super) struct UnspawnedMobEditor<'a>(pub std::marker::PhantomData<&'a mut ()>);
    pub(super) struct SpawnGroup;
}
