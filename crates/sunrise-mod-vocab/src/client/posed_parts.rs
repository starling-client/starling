//! Borrowed parts of a living model before, during and after posing.

// no wire: in-process model part selector
/// A named humanoid part, as vanilla's `PlayerModel` names it. The outer skin parts (`Hat`,
/// `Jacket`, the sleeves and the pants) are children of the parts they cover. Use
/// [`Self::model_name`] to look it up on a [`FrameModel`]; the enum is not a model index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HumanoidPart {
    /// The head.
    Head,
    /// The hat layer, over the head.
    Hat,
    /// The body.
    Body,
    /// The jacket layer, over the body.
    Jacket,
    /// The right arm.
    RightArm,
    /// The right sleeve, over the right arm.
    RightSleeve,
    /// The left arm.
    LeftArm,
    /// The left sleeve, over the left arm.
    LeftSleeve,
    /// The right leg.
    RightLeg,
    /// The right trouser leg, over the right leg.
    RightPants,
    /// The left leg.
    LeftLeg,
    /// The left trouser leg, over the left leg.
    LeftPants,
}

impl HumanoidPart {
    /// Every part, parents before their children.
    pub const ALL: [Self; 12] = [
        Self::Head,
        Self::Hat,
        Self::Body,
        Self::Jacket,
        Self::RightArm,
        Self::RightSleeve,
        Self::LeftArm,
        Self::LeftSleeve,
        Self::RightLeg,
        Self::RightPants,
        Self::LeftLeg,
        Self::LeftPants,
    ];

    /// Its name in the model (`right_sleeve`).
    pub fn model_name(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Hat => "hat",
            Self::Body => "body",
            Self::Jacket => "jacket",
            Self::RightArm => "right_arm",
            Self::RightSleeve => "right_sleeve",
            Self::LeftArm => "left_arm",
            Self::LeftSleeve => "left_sleeve",
            Self::RightLeg => "right_leg",
            Self::RightPants => "right_pants",
            Self::LeftLeg => "left_leg",
            Self::LeftPants => "left_pants",
        }
    }
}

// no wire: borrows the baked hierarchy and pose for this draw.
/// The selected living model's baked hierarchy, including resource-pack parts. Pre sees the
/// bind pose (before animation and skin visibility); registered layers and Post see the final
/// pose and effective visibility. The hierarchy and name indices are resolved when the model
/// is baked. Borrowed parts and their indices are scoped to this callback: a later draw may
/// select a different model.
pub trait FrameModel {
    /// The root of this model, even when it has no cubes of its own.
    fn root(&self) -> FrameModelPart<'_>;

    /// The first depth-first match for this exact name, including the root and hidden parts.
    /// Returns `None` if there is no match. Follow hierarchy links with [`Self::part_at`] to
    /// distinguish parts with the same name.
    fn part(&self, name: &str) -> Option<FrameModelPart<'_>>;

    /// The part at a model-local index, or `None` if it is outside this model.
    fn part_at(&self, index: usize) -> Option<FrameModelPart<'_>>;
}

// no wire: borrows the selected hierarchy and draw-local visibility.
/// The selected body in a Pre callback, before animation. Player Pre and Living Pre edit the same
/// model, in that order. Each later listener sees the earlier hides. Those hides apply to the
/// body and layers drawn on it after animation and skin visibility, not to independent draws
/// such as armour, held items or capes. They never persist to another draw.
pub trait FrameModelEdit: FrameModel {
    /// Hides the part at this model-local index and its descendants. Returns `false` without
    /// making a change if the index is outside this model. Hiding a part again succeeds.
    fn hide(&mut self, index: usize) -> bool;
}

// no wire: borrows the posed hierarchy and pose for this callback.
/// A posed model in a `living_model_posed` callback: the model after vanilla's last posing step
/// (the RETURN of `EntityModel.setupAnim`), before it draws. Every [`FrameModel`] read shows the
/// pose so far, earlier listeners' edits included. Edits last for this draw only, and a
/// part is addressed by its model-local index, which [`FrameModel::part`] resolves.
pub trait FrameModelPoseEdit: FrameModel {
    /// Replaces the local pose of the part at this index: position in texels, rotation in
    /// radians and scale as unitless factors, all relative to the parent, as
    /// `ModelPart.setPos`, `setRotation` and `xScale`/`yScale`/`zScale` leave them. Returns
    /// `false` without a change if the index is outside this model or any component of `pose` is
    /// not finite: the part keeps its pose.
    fn set_pose(&mut self, index: usize, pose: crate::PartPose) -> bool;

    /// Restores the bind pose of the part at this index (`ModelPart.resetPose`): the pose it
    /// has before any animation. Returns `false` if the index is outside this model.
    fn reset_pose(&mut self, index: usize) -> bool;
}

// no wire: name and local pose are borrowed from the current frame model.
/// A read-only view of one part within a [`FrameModel`]. Hierarchy links are indices for
/// [`FrameModel::part_at`]; a missing link is `None`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct FrameModelPart<'a> {
    /// This part's index in the model.
    pub index: usize,
    /// Baked name of this part.
    pub name: &'a str,
    /// The parent, absent for the root.
    pub parent: Option<usize>,
    /// The first child, absent for a leaf.
    pub first_child: Option<usize>,
    /// The next child of the same parent, absent for the last sibling and for the root.
    pub next_sibling: Option<usize>,
    /// Position (x, y, z) in texels, then rotation (x, y, z) in radians, then scale (x, y, z)
    /// as unitless factors, all relative to the parent. The root is relative to the model origin.
    pub local_pose: &'a [f32; 9],
    /// Whether visibility allows traversal of this subtree: false if this part or an ancestor
    /// is hidden. A visible part can still submit no cubes of its own (an empty part, a part
    /// whose cubes were skipped, or a draw that submitted only layers).
    pub visible: bool,
}

// hidden: the engine lends a view of a node in its selected frame model.
#[doc(hidden)]
pub fn frame_model_part<'a>(
    index: usize,
    name: &'a str,
    parent: Option<usize>,
    first_child: Option<usize>,
    next_sibling: Option<usize>,
    local_pose: &'a [f32; 9],
    visible: bool,
) -> FrameModelPart<'a> {
    FrameModelPart {
        index,
        name,
        parent,
        first_child,
        next_sibling,
        local_pose,
        visible,
    }
}

impl FrameModelPart<'_> {
    /// Copies the nine local components into a [`crate::PartPose`]. This is bind pose in Pre
    /// and final pose in a layer or Post. Follow the parent links and compose the transforms
    /// from the root down when attaching geometry to a nested part.
    pub fn pose(&self) -> crate::PartPose {
        let &[x, y, z, x_rot, y_rot, z_rot, x_scale, y_scale, z_scale] = self.local_pose;
        crate::PartPose {
            x,
            y,
            z,
            x_rot,
            y_rot,
            z_rot,
            x_scale,
            y_scale,
            z_scale,
        }
    }
}

// no wire: lends the live pose of this setup-animation invocation.
/// The live local pose borrowed during `HumanoidModel.setupAnim`. The head, hat, body,
/// arms and legs can be posed here if present. Later listeners see each edit immediately;
/// vanilla's remaining animation steps can still change the edited components. The
/// view is valid only for the current callback. Scale and visibility are not writable here.
pub trait HumanoidPose {
    /// The local [position, rotation] of a base part. Missing parts, sleeves, the
    /// jacket and pants return `None`.
    fn get(&self, part: HumanoidPart) -> Option<[[f32; 3]; 2]>;

    /// Writes local position components in texels directly, matching `HumanoidModel.setupAnim`
    /// without filtering component values. Returns `false` only for a missing or noneditable part.
    fn set_position(&mut self, part: HumanoidPart, position: [f32; 3]) -> bool;

    /// Writes local rotation components in radians directly, without clamping or filtering
    /// component values. Returns `false` only for a missing or noneditable part.
    fn set_rotation(&mut self, part: HumanoidPart, rotation: [f32; 3]) -> bool;
}
