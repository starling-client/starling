pub struct ConfigurationTaskRegistration {
    tasks: Vec<Box<dyn Send>>,
}

impl ConfigurationTaskRegistration {
    pub fn checkpoint(&self) -> (usize, bool) {
        (self.tasks.len(), false)
    }

    pub fn rollback(&mut self, (len, _): (usize, bool)) {
        self.tasks.truncate(len);
    }
}
