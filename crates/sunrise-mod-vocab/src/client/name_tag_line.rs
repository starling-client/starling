//! Which line of a name tag `render_name_tag_do_render` is asked about.

/// A line of an entity's name tag, in the order vanilla draws them (`EntityRenderer.submitNameDisplay`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: passed by value to a local client render hook.
pub enum NameTagLine {
    /// `scoreText`, the lower line: the below-name objective's score, or a mannequin's description.
    Score,
    /// `nameTag`, the entity's name: lifted one line (`9 * 1.15 * 0.025`) when a score line exists.
    Name,
}
