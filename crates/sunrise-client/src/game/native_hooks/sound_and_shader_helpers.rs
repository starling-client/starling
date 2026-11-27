/// Whether `id` is a sound event's id, `namespace:path` as `Identifier`
/// allows: the namespace of `[a-z0-9_.-]`, the path also with `/`.
pub(in crate::game) fn sound_id(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    let allowed = |c: char, slash: bool| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c) || (slash && c == '/')
    };
    !namespace.is_empty()
        && !path.is_empty()
        && namespace.chars().all(|c| allowed(c, false))
        && path.chars().all(|c| allowed(c, true))
}

/// `params` with each value that isn't finite as 0, so a mod's mistake can't
/// put a NaN in every pixel of a pack's picture.
pub(super) fn finite_params(params: [[f32; 4]; 8]) -> [[f32; 4]; 8] {
    params.map(|v| v.map(|x| if x.is_finite() { x } else { 0.0 }))
}
