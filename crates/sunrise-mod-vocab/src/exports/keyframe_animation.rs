use client::keyframe_animation::JsonAnimationTypeRegistration;
pub use client::keyframe_animation::{
    AnimationChannel, AnimationHolder, AnimationInterpolationFn, AnimationState, AnimationTarget,
    AnimationTargetApply, AnimationTargetConvert, BoundAnimationTarget, BoundInterpolation,
    Interpolation, Keyframe, KeyframeAnimation,
};
mod __private {
    mod game {
        pub use crate::client::keyframe_animation::{
            JsonAnimationTypeRegistration, bind_animation_interpolation, bind_animation_target,
            convert_animation_target,
        };
    }
}
