//! Capture and dispatch terrain visibility on the client world thread. Workers receive only
//! the checked plan and the same immutable section data used for the capture.

use super::Game;
use mod_api::{Answered, SectionVisibility, SectionVisibilityRefusal};
use sunrise_client::mesher::SectionPlanBuilder;

impl Game<'_> {
    /// Ask mods about the admitted requests and submit their combined answers to the workers.
    pub(in crate::game) fn prepare_section_visibility(&mut self) {
        let requests = self.streaming.take_visibility_requests();
        if requests.is_empty() {
            return;
        }
        let Ok(index) = crate::game::api_values::block_state::current() else {
            for request in requests {
                self.streaming
                    .submit_visibility_answer(&mut self.world, request, None);
            }
            return;
        };
        let Ok(air) = index.to_api(self.world.air) else {
            for request in requests {
                self.streaming
                    .submit_visibility_answer(&mut self.world, request, None);
            }
            return;
        };
        let poisoned = mod_api::__private::poisoned_listeners(mod_api::HookId::SectionVisibility);
        let mut refusals = std::mem::take(&mut self.streaming.section_visibility.refusals);
        for request in requests {
            let input = request.capture_input(|raw| {
                index
                    .to_api(sunrise_world::registry::BlockStateId(raw))
                    .unwrap_or(air)
            });
            let mut plan = SectionPlanBuilder::new();
            self.native_section_visibility(mod_api::__private::SectionVisibilityCall {
                input: &input,
                answers: &mut |Answered { answer, mod_id, .. }: Answered<SectionVisibility>| {
                    if let Err(refusal) = plan.apply(&answer)
                        && refusals.insert((mod_id, refusal.code()))
                    {
                        log::warn!(
                            "Sunrise: mod {mod_id}'s section visibility at {:?} refused whole: {refusal}",
                            input.origin
                        );
                    }
                },
            });
            if mod_api::__private::poisoned_listeners(mod_api::HookId::SectionVisibility)
                != poisoned
            {
                // Every pending result, including meshes accepted earlier this frame, might have
                // included the failed listener. Reset the policy before anything can publish.
                log::warn!(
                    "Sunrise: a section_visibility listener failed ({}); it is switched off for \
                     the session and sections are asked again without it",
                    SectionVisibilityRefusal::Failed
                );
                self.streaming
                    .refresh_section_visibility(&mut self.world, true);
                return;
            }
            self.streaming
                .submit_visibility_answer(&mut self.world, request, plan.finish());
        }
        self.streaming.section_visibility.refusals = refusals;
    }
}
