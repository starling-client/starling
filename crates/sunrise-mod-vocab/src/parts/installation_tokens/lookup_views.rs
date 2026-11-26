//! Narrow cold lookup views for API-owned registration tests.
/// Installed controller ids, tokens and validator presence, without lending the callbacks.
#[cfg(any(test, feature = "testing"))]
pub fn testing_controllers() -> Vec<(&'static str, u64, bool)> {
    crate::parts::installed_controllers()
        .iter()
        .map(|controller| {
            (
                &*controller.id,
                controller.token,
                controller.validator.is_some(),
            )
        })
        .collect()
}
/// The immutable command argument declarations, copied without exposing Installed.
#[cfg(any(test, feature = "testing"))]
pub fn testing_command_arguments() -> Vec<super::CommandArgumentInfoInput> {
    crate::parts::installed_command_arguments()
        .iter()
        .map(Into::into)
        .collect()
}
/// The immutable entity selector declarations, copied without exposing Installed.
#[cfg(any(test, feature = "testing"))]
pub fn testing_entity_selectors() -> Vec<super::EntitySelectorInfoInput> {
    crate::parts::installed_entity_selectors()
        .iter()
        .map(Into::into)
        .collect()
}
/// Probe only whether a hook may access Parts in the current game context.
#[cfg(any(test, feature = "testing"))]
pub fn check_parts(
    hook: crate::HookId,
    name: &'static str,
) -> Result<(), crate::parts::MissingResolvedParts> {
    crate::parts::try_parts(hook, name).map(|_| ())
}
/// Scope an empty pose lookup, including on transports that do not consult it.
#[must_use]
#[cfg(any(test, feature = "testing"))]
pub fn empty_pose_table() -> impl Sized {
    (
        crate::client::arm_pose::test_table(Vec::new()),
        std::marker::PhantomData::<std::rc::Rc<()>>,
    )
}
