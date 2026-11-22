impl LevelRef {
    fn block(self, _: &dyn ServerHost, _: [i32; 3]) -> Option<BlockKey> {
        None
    }
}
