mod api {
    pub struct CustomSpawnersEditor {
        spawners: Vec<Box<dyn Send>>,
    }

    impl CustomSpawnersEditor {
        pub fn checkpoint(&self) -> usize {
            self.spawners.len()
        }

        pub fn rollback(&mut self, len: usize) {
            self.spawners.truncate(len);
        }
    }
}
