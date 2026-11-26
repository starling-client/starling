//! The service fakes: the commands a mod runs and the messages it sends.
use crate::{CommandFeedback, CommandResult, Commands, Messages, Text, TextFormError};
use std::{cell::RefCell, collections::HashSet, sync::mpsc::Sender};

// no wire: in-process testing harness state, not a mod boundary payload.
/// The server's command service: the commands run, in order, of which those listed as known
/// succeed, and the feedback and failure lines given.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FakeCommands {
    known: HashSet<String>,
    /// Each command line run, in order.
    pub ran: Vec<String>,
    /// Each feedback line given, in order.
    pub feedback: Vec<Text>,
    /// Each feedback line's `broadcast_to_admins`, in the same order.
    pub feedback_broadcast: Vec<bool>,
    /// Each failure line given, in order.
    pub failure: Vec<Text>,
}

impl FakeCommands {
    /// Makes command line `line` succeed when run.
    pub fn known(&mut self, line: &str) -> &mut Self {
        self.known.insert(line.to_owned());
        self
    }
}

impl Commands for FakeCommands {
    fn run_command(&mut self, line: &str) -> bool {
        self.ran.push(line.to_owned());
        self.known.contains(line)
    }
    fn run_command_with_output(&mut self, line: &str) -> CommandResult {
        CommandResult::new(self.run_command(line), Vec::new())
    }
    fn run_command_with_feedback(
        &mut self,
        line: &str,
        _: Sender<Result<CommandFeedback, TextFormError>>,
    ) -> bool {
        self.run_command(line)
    }
    fn feedback(
        &mut self,
        line: Text,
        broadcast_to_admins: bool,
    ) -> Result<(), crate::text::TextFormError> {
        line.validate_depth()?;
        self.feedback.push(line);
        self.feedback_broadcast.push(broadcast_to_admins);
        Ok(())
    }
    fn failure(&mut self, line: Text) -> Result<(), crate::text::TextFormError> {
        line.validate_depth()?;
        self.failure.push(line);
        Ok(())
    }
}

// no wire: in-process testing harness state, not a mod boundary payload.
/// The server's message service: the lines broadcast, in order.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FakeMessages {
    broadcast: RefCell<Vec<Text>>,
}

impl FakeMessages {
    /// Each line broadcast, in order.
    pub fn broadcast_lines(&self) -> Vec<Text> {
        self.broadcast.borrow().clone()
    }
}

impl Messages for FakeMessages {
    fn broadcast(&self, line: Text) -> Result<(), crate::text::TextFormError> {
        line.validate_depth()?;
        self.broadcast.borrow_mut().push(line);
        Ok(())
    }
}
