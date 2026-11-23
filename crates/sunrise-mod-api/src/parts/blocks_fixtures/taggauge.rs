/// A comparator reading of 100 more, for the blocks in tag `test:gauged`.
pub(crate) struct TagGauge;
#[mod_api::sunrise_part]
impl BlockPart for TagGauge {
    fn analog_output(&self, _: &dyn LevelHost, event: &mut AnalogOutputEvent) {
        event.output += 100;
    }
}
