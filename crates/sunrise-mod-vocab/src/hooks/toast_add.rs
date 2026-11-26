//! Client toast admission (`ToastAddEvent`).

pub trait Hooks {
    /// A client toast is about to enter its queue (`ToastAddEvent`). Return
    /// `Flow::Cancel` to discard only this candidate, before slot allocation,
    /// animation or sound. `kind` identifies its family; music is `Other`.
    /// Replace `title` and `description` to change the displayed components.
    /// For advancements the title is the frame label and the description is
    /// the advancement name. Existing recipe/system toasts updated in place
    /// do not fire another add event. Requests from `ClientHost::add_toast` enter as
    /// `ToastKind::Mod`, including requests that would replace an existing mod toast.
    /// Integrated-server error notices are new system toast candidates on the owning
    /// client, not on the server or its guests.
    /// Pending chunk I/O reports stay with that window when the world closes.
    /// Client-lib and title producers prepare candidates for the game to admit
    /// with a `ClientHost` before applying.
    /// Diagnostics naming a mod's panicking callback are not candidates for
    /// this hook and cannot be edited or cancelled.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "cancel",
        neoforge = "ToastAddEvent",
        cancellable = true,
        version = 3
    )]
    fn toast_add(
        kind: mod_api::ToastKind,
        title: &mut Text,
        description: &mut Option<Text>,
        cancel: &mut bool,
    );
}
