//! LocalPlayer sprint decisions around KeyboardInput.tick. Conditions are the
//! current player queries; setting sprint dispatches to the live attribute state.
use crate::input;

#[derive(Clone, Copy, Debug, Default)]
pub struct Keys {
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub shift: bool,
    pub sprint: bool,
    /// A sampled stick or listener-edited move vector (`[left, forward]`),
    /// used instead of deriving the direction from the keys.
    pub analog: Option<[f32; 2]>,
}
impl Keys {
    pub fn direction(self) -> [f32; 2] {
        match self.analog {
            Some(v) => v,
            None => input::keyboard_direction(self.forward, self.backward, self.left, self.right),
        }
    }
    /// Add a stick's move vector: past a third of its travel it presses the keys of the
    /// nearest of their eight directions, which the code reading keys sees (sprint, vehicles,
    /// the server's input packet, and a server's movement: `analog` cleared).
    pub fn with_stick(mut self, stick: Option<[f32; 2]>) -> Self {
        if let Some([left, forward]) = stick {
            if left.hypot(forward) > 0.3 {
                // 0 straight ahead, a quarter turn to the left.
                let eighth = (left.atan2(forward) / std::f32::consts::FRAC_PI_4).round() as i32;
                let (f, l) = [
                    (1, 0),
                    (1, 1),
                    (0, 1),
                    (-1, 1),
                    (-1, 0),
                    (-1, -1),
                    (0, -1),
                    (1, -1),
                ][eighth.rem_euclid(8) as usize];
                self.forward |= f > 0;
                self.backward |= f < 0;
                self.left |= l > 0;
                self.right |= l < 0;
            }
            self.analog = Some([left, forward]);
        }
        self
    }
}

#[cfg(test)]
mod stick_tests {
    use super::Keys;

    fn pressed(stick: [f32; 2]) -> (bool, bool, bool, bool) {
        let k = Keys::default().with_stick(Some(stick));
        (k.forward, k.backward, k.left, k.right)
    }

    #[test]
    fn a_stick_presses_the_keys_of_the_nearest_of_eight_directions() {
        // [left, forward]: a half-tilt diagonal is a diagonal, however far it is pushed.
        assert_eq!(pressed([0.0, 0.6]), (true, false, false, false));
        assert_eq!(pressed([-0.3, 0.34]), (true, false, false, true));
        assert_eq!(pressed([0.5, 0.1]), (false, false, true, false));
        assert_eq!(pressed([0.4, -0.45]), (false, true, true, false));
        assert_eq!(pressed([0.0, -1.0]), (false, true, false, false));
        // Within a third of its travel it presses nothing.
        assert_eq!(pressed([0.15, 0.2]), (false, false, false, false));
        // 20 degrees off ahead is still ahead; 25 degrees is the diagonal.
        let at = |deg: f32| [deg.to_radians().sin(), deg.to_radians().cos()];
        assert_eq!(pressed(at(20.0)), (true, false, false, false));
        assert_eq!(pressed(at(25.0)), (true, false, true, false));
    }

    #[test]
    fn keys_move_the_eight_ways_at_full_speed_once_the_analog_vector_is_gone() {
        let mut k = Keys::default().with_stick(Some([-0.3, 0.34]));
        assert_eq!(k.direction(), [-0.3, 0.34]);
        k.analog = None;
        let [l, f] = k.direction();
        assert!(
            (l.hypot(f) - 1.0).abs() < 1e-4 && (l + f).abs() < 1e-4,
            "{l} {f}"
        );
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SprintState {
    pub trigger_time: i32,
    pub previous_forward: bool,
    pub previous_shift: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Conditions {
    pub sprinting: bool,
    pub mobility_restricted: bool,
    pub passenger: bool,
    pub vehicle_can_sprint: bool,
    pub vehicle_local: bool,
    pub enough_food: bool,
    pub may_fly: bool,
    pub flying: bool,
    pub shallow_water: bool,
    pub slow_item: bool,
    pub fall_flying: bool,
    pub under_water: bool,
    pub moving_slowly: bool,
    pub swimming: bool,
    pub in_water: bool,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub minor_horizontal_collision: bool,
}
impl Conditions {
    fn possible(self, allow_shallow_water: bool, shallow_water: impl FnOnce(bool) -> bool) -> bool {
        !self.mobility_restricted
            && if self.passenger {
                self.vehicle_can_sprint && self.vehicle_local
            } else {
                self.enough_food || self.may_fly
            }
            && (allow_shallow_water || !shallow_water(self.shallow_water))
    }
    fn can_start(self, forward: bool, shallow_water: impl FnOnce(bool) -> bool) -> bool {
        !self.sprinting
            && forward
            && self.possible(self.flying, shallow_water)
            && !self.slow_item
            && (!self.fall_flying || self.under_water)
            && (!self.moving_slowly || self.under_water)
    }
    fn should_stop(
        self,
        forward: bool,
        shift: bool,
        shallow_water: impl FnOnce(bool) -> bool,
    ) -> bool {
        if self.swimming {
            !self.possible(true, shallow_water)
                || !self.in_water
                || (!forward && !self.on_ground && !shift)
        } else {
            !self.possible(self.flying, shallow_water)
                || !forward
                || (self.horizontal_collision && !self.minor_horizontal_collision)
        }
    }
}
impl SprintState {
    /// `LocalPlayer.aiStep`'s sprint phase.
    /// The default vanilla window is seven ticks; options can select 0..10.
    /// The setter runs for every native request, including repeated true requests.
    pub fn tick<E>(
        &mut self,
        keys: Keys,
        conditions: Conditions,
        window: i32,
        mut set_sprinting: impl FnMut(bool) -> Result<(), E>,
    ) -> Result<(), E> {
        self.tick_with_context(
            keys,
            conditions,
            window,
            &mut (),
            |_, vanilla| vanilla,
            |_, value| set_sprinting(value),
        )
    }

    /// Run each shallow-water query at its own `LocalPlayer.isSprintingPossible` check.
    /// A setter takes effect before a later query in the same tick.
    pub fn tick_with_context<C, E>(
        &mut self,
        keys: Keys,
        conditions: Conditions,
        window: i32,
        context: &mut C,
        mut shallow_water: impl FnMut(&C, bool) -> bool,
        mut set_sprinting: impl FnMut(&mut C, bool) -> Result<(), E>,
    ) -> Result<(), E> {
        if self.trigger_time > 0 {
            self.trigger_time -= 1;
        }
        let old_forward = self.previous_forward;
        let old_shift = self.previous_shift;
        let forward = keys.direction()[1] > 1.0e-5;
        self.previous_forward = forward;
        self.previous_shift = keys.shift;
        if old_shift || (conditions.slow_item && !conditions.passenger) || keys.backward {
            self.trigger_time = 0;
        }
        let mut sprinting = conditions.sprinting;
        if conditions.can_start(forward, |vanilla| shallow_water(context, vanilla)) {
            if !old_forward {
                if self.trigger_time > 0 {
                    set_sprinting(context, true)?;
                    sprinting = true;
                } else {
                    // `Options.sprintWindow` uses `IntRange(0, 10)`.
                    self.trigger_time = window.clamp(0, 10);
                }
            }
            if keys.sprint {
                set_sprinting(context, true)?;
                sprinting = true;
            }
        }
        if sprinting
            && conditions.should_stop(forward, keys.shift, |vanilla| {
                shallow_water(context, vanilla)
            })
        {
            set_sprinting(context, false)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing() -> Conditions {
        Conditions {
            enough_food: true,
            on_ground: true,
            ..Conditions::default()
        }
    }

    fn step(state: &mut SprintState, keys: Keys, sprinting: &mut bool) {
        let current = *sprinting;
        let mut next = current;
        state
            .tick(
                keys,
                Conditions {
                    sprinting: current,
                    ..standing()
                },
                7,
                |value| {
                    next = value;
                    Ok::<(), ()>(())
                },
            )
            .unwrap();
        *sprinting = next;
    }

    /// Vanilla sprint is a state. Releasing the sprint key, including on the
    /// jump tick, keeps it while forward is held; releasing forward stops it.
    #[test]
    fn sprint_survives_releasing_the_key_through_a_jump() {
        let mut state = SprintState::default();
        let mut sprinting = false;
        step(
            &mut state,
            Keys {
                forward: true,
                sprint: true,
                ..Keys::default()
            },
            &mut sprinting,
        );
        assert!(sprinting);
        step(
            &mut state,
            Keys {
                forward: true,
                jump: true,
                ..Keys::default()
            },
            &mut sprinting,
        );
        assert!(
            sprinting,
            "a jump after the sprint key comes up must stay a sprint"
        );
        step(
            &mut state,
            Keys {
                jump: true,
                ..Keys::default()
            },
            &mut sprinting,
        );
        assert!(!sprinting);
    }
}
