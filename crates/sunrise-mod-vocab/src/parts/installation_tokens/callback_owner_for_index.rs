//! Construct an indexed retained-callback owner during registration.
pub fn callback_owner_for_index(
    id: &'static str,
    index: usize,
) -> crate::__private::game::CallbackOwner {
    crate::callback_owner::CallbackOwner::for_index(id, index)
}
