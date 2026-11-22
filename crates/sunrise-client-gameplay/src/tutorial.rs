//! Client tutorial progression (`net.minecraft.client.tutorial`).

/// The numeric values are the order in `options_file::TUTORIAL_STEPS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Movement = 0,
    FindTree = 1,
    PunchTree = 2,
    OpenInventory = 3,
    CraftPlanks = 4,
    None = 5,
}

impl Step {
    pub fn from_option(value: i32) -> Self {
        match value {
            0 => Self::Movement,
            1 => Self::FindTree,
            2 => Self::PunchTree,
            3 => Self::OpenInventory,
            4 => Self::CraftPlanks,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    Move,
    Look,
    FindTree,
    PunchTree,
    OpenInventory,
    CraftPlanks,
    SocialInteractions,
}

/// The observations needed on the first tick of each step.
#[derive(Default)]
pub struct Previous {
    pub tree_item: bool,
    pub log: bool,
    pub plank: bool,
    pub mined_tree: bool,
    pub crafted_plank: bool,
}

pub struct Tutorial {
    pub step: Step,
    pub waiting: u32,
    moved: bool,
    turned: bool,
    move_ticks: u32,
    look_ticks: u32,
    move_finished: Option<u32>,
    look_finished: bool,
    reset_count: u32,
    pub break_progress: f32,
    cancelled_hint: Option<Hint>,
}

impl Tutorial {
    pub fn new(step: Step) -> Self {
        Self {
            step,
            waiting: 0,
            moved: false,
            turned: false,
            move_ticks: 0,
            look_ticks: 0,
            move_finished: None,
            look_finished: false,
            reset_count: 0,
            break_progress: 0.0,
            cancelled_hint: None,
        }
    }

    /// Returns whether the persisted step changed.
    fn set_step(&mut self, step: Step) -> bool {
        if self.step == step {
            return false;
        }
        *self = Self::new(step);
        true
    }

    pub fn input(&mut self) {
        self.moved = true;
    }
    pub fn mouse(&mut self, x: f64, y: f64) {
        self.turned |= x.abs() > 0.01 || y.abs() > 0.01;
    }

    pub fn tick(&mut self, survival: bool, previous: &Previous) -> bool {
        self.waiting = self.waiting.saturating_add(1);
        if self.step == Step::Movement {
            self.move_ticks += u32::from(std::mem::take(&mut self.moved));
            self.look_ticks += u32::from(std::mem::take(&mut self.turned));
            if self.move_ticks > 40 && self.move_finished.is_none() {
                self.move_finished = Some(self.waiting);
            }
            self.look_finished |= self.look_ticks > 40;
            if self.move_finished.is_some() && self.look_finished {
                return self.set_step(if survival { Step::FindTree } else { Step::None });
            }
        } else if self.step != Step::None && !survival {
            return self.set_step(Step::None);
        } else if self.waiting == 1 {
            match self.step {
                Step::FindTree if previous.tree_item || previous.mined_tree => {
                    return self.set_step(Step::CraftPlanks);
                }
                Step::PunchTree if previous.log || previous.mined_tree => {
                    return self.set_step(Step::CraftPlanks);
                }
                Step::CraftPlanks if previous.plank || previous.crafted_plank => {
                    return self.set_step(Step::None);
                }
                _ => {}
            }
        }
        false
    }

    pub fn look_at_tree(&mut self) -> bool {
        self.step == Step::FindTree && self.set_step(Step::PunchTree)
    }
    pub fn destroy(&mut self, is_log: bool, progress: f32) -> bool {
        if self.step != Step::PunchTree {
            return false;
        }
        if is_log && progress > 0.0 {
            self.break_progress = progress;
            if progress >= 1.0 {
                return self.set_step(Step::OpenInventory);
            }
        } else {
            self.break_progress = 0.0;
            if is_log {
                self.reset_count = self.reset_count.saturating_add(1);
            }
        }
        false
    }
    pub fn open_inventory(&mut self) -> bool {
        self.step == Step::OpenInventory && self.set_step(Step::CraftPlanks)
    }
    pub fn get_item(&mut self, tree: bool, log: bool, plank: bool) -> bool {
        match self.step {
            Step::FindTree if tree => self.set_step(Step::CraftPlanks),
            Step::PunchTree if log => self.set_step(Step::CraftPlanks),
            Step::CraftPlanks if plank => self.set_step(Step::None),
            _ => false,
        }
    }
    pub fn hint(&self) -> Option<(Hint, Option<f32>)> {
        match self.step {
            Step::Movement if self.waiting >= 100 && self.move_finished.is_none() => {
                Some((Hint::Move, Some(self.move_ticks as f32 / 40.0)))
            }
            Step::Movement
                if !self.look_finished
                    && self
                        .move_finished
                        .is_some_and(|t| self.waiting.saturating_sub(t) >= 20)
                    && self.waiting >= 100 =>
            {
                Some((Hint::Look, Some(self.look_ticks as f32 / 40.0)))
            }
            Step::FindTree if self.waiting >= 6000 => Some((Hint::FindTree, None)),
            Step::PunchTree if self.waiting >= 600 || self.reset_count > 3 => {
                Some((Hint::PunchTree, Some(self.break_progress)))
            }
            Step::OpenInventory if self.waiting >= 600 => Some((Hint::OpenInventory, None)),
            Step::CraftPlanks if self.waiting >= 1200 => Some((Hint::CraftPlanks, None)),
            _ => None,
        }
    }

    pub fn cancelled_hint(&self) -> Option<Hint> {
        self.cancelled_hint
    }
    pub fn set_cancelled_hint(&mut self, hint: Option<Hint>) {
        self.cancelled_hint = hint;
    }
}

/// TutorialToast.update uses the time fully visible (not the slide-in time),
/// and hides only after 8000 ms. Its timed bar does not use progress smoothing.
pub fn social_hint_progress(fully_visible_ms: i64) -> (bool, f32) {
    (
        fully_visible_ms <= 8000,
        (fully_visible_ms as f32 / 8000.0).min(1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn social_hint_fills_for_eight_seconds_before_hiding() {
        assert_eq!(social_hint_progress(0), (true, 0.0));
        assert_eq!(social_hint_progress(4000), (true, 0.5));
        assert_eq!(social_hint_progress(8000), (true, 1.0));
        assert_eq!(social_hint_progress(8001), (false, 1.0));
    }

    #[test]
    fn social_hint_counts_from_full_visibility_and_finishes_sliding_out() {
        let mut slide = sunrise_client::toasts::Slide::default();
        for (now, visible) in [(0, true), (600, true), (8600, true), (8601, false)] {
            let (_, finished) = slide.update(now, |fully| social_hint_progress(fully).0);
            assert!(!finished);
            assert_eq!(social_hint_progress(slide.fully_for).0, visible);
        }
        let (_, finished) = slide.update(9202, |fully| social_hint_progress(fully).0);
        assert!(finished);
    }

    #[test]
    fn craft_planks_hint_toast_appears_after_delay_and_clears_on_planks() {
        let mut tutorial = Tutorial::new(Step::CraftPlanks);
        tutorial.waiting = 1199;
        assert_eq!(tutorial.hint(), None);

        tutorial.waiting = 1200;
        assert_eq!(tutorial.hint(), Some((Hint::CraftPlanks, None)));

        // Receiving an unrelated item (e.g. log or stick) does not complete the step
        let changed = tutorial.get_item(false, true, false);
        assert!(!changed);
        assert_eq!(tutorial.step, Step::CraftPlanks);
        assert_eq!(tutorial.hint(), Some((Hint::CraftPlanks, None)));

        // Receiving planks completes the tutorial step and dismisses the hint toast
        let changed = tutorial.get_item(false, false, true);
        assert!(changed);
        assert_eq!(tutorial.step, Step::None);
        assert_eq!(tutorial.hint(), None);
    }

    #[test]
    fn initial_tick_with_planks_clears_craft_planks_step() {
        let mut tutorial = Tutorial::new(Step::CraftPlanks);
        let previous = Previous {
            plank: true,
            ..Default::default()
        };
        assert!(tutorial.tick(true, &previous));
        assert_eq!(tutorial.step, Step::None);
        assert_eq!(tutorial.hint(), None);
    }
}
