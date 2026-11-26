//! Native access to the server console: operator-visible logs and command source output.
use super::{Game, commands::execute::Source};
use mod_api::{CommandFeedback, CommandResult, TextFormError};
use std::sync::{Arc, Mutex, Weak, mpsc::Sender};
use sunrise_client::{logging, text::Text};

pub(in crate::game) enum Output {
    Plain,
    Stream(Sender<Result<CommandFeedback, TextFormError>>),
}

/// Output sink owned by a native console command source.
#[derive(Clone, Debug)]
pub(in crate::game) enum ConsoleOutput {
    Silent,
    Plain(Weak<Mutex<Vec<String>>>),
    Stream(Sender<Result<CommandFeedback, TextFormError>>),
}

impl ConsoleOutput {
    pub(in crate::game) fn send(&self, kind: fn(mod_api::Text) -> CommandFeedback, text: &Text) {
        match self {
            Self::Silent => {}
            Self::Plain(lines) => {
                if let Some(lines) = lines.upgrade()
                    && let Ok(mut lines) = lines.lock()
                {
                    lines.push(text.plain());
                }
            }
            Self::Stream(output) => {
                // Only a receiver that has stopped listening can refuse this send.
                let _ = output.send(sunrise_client_lib::api_text::to_api(text).map(kind));
            }
        }
    }
}

impl Game<'_> {
    /// `console_line` for each line logged since the last server tick, while
    /// an enabled mod listens; the lines its listeners log are not kept.
    pub(super) fn native_console_lines(&mut self) {
        let listened = mod_api::__private::enabled_listened(mod_api::HookId::ConsoleLine);
        self.dispatch_console_lines(logging::take_tapped(listened));
    }

    pub(super) fn dispatch_console_lines(&mut self, lines: Vec<logging::TappedLine>) {
        for line in lines {
            let level = line.level.as_str();
            logging::untapped(|| {
                self.native_console_line(level, &line.thread, &line.message, line.dropped);
            });
        }
    }

    /// `Commands::run_command`: run as the console with no source output.
    pub(super) fn native_run_command(&mut self, line: &str) -> bool {
        self.run_console_command(line, None).success
    }

    /// `Commands::run_command_with_output`: collect the console source's output as plain text.
    pub(super) fn native_run_command_with_output(&mut self, line: &str) -> CommandResult {
        self.run_console_command(line, Some(Output::Plain))
    }

    /// Run at owner permission from the world spawn level (`MinecraftServer.createCommandSourceStack`).
    /// Nested runs use their own source and output, leaving the calling command's execution state,
    /// result callbacks and output untouched.
    pub(in crate::game) fn run_console_command(
        &mut self,
        line: &str,
        output: Option<Output>,
    ) -> CommandResult {
        if self.lan.is_none() {
            return CommandResult::default();
        }
        let line = if output.is_some() {
            let line = line.trim();
            let line = line.strip_prefix('/').unwrap_or(line);
            if line.trim().is_empty() {
                return CommandResult::default();
            }
            line
        } else {
            line
        };
        let (sink, plain_lines) = match output {
            None => (ConsoleOutput::Silent, None),
            Some(Output::Plain) => {
                let lines = Arc::new(Mutex::new(Vec::new()));
                let weak = Arc::downgrade(&lines);
                (ConsoleOutput::Plain(weak), Some(lines))
            }
            Some(Output::Stream(sender)) => (ConsoleOutput::Stream(sender), None),
        };
        let silent = matches!(sink, ConsoleOutput::Silent);
        let filter = std::mem::replace(&mut self.native_command_filter, true);
        let outer_exec = std::mem::take(&mut self.exec);
        let outer_results = self.cmds.results.take();
        let outer_chat = self.chat_capture.take();
        let success = self.as_console_in_spawn_level(|g| {
            let source = Source {
                silent,
                native_output: Some(sink),
                ..g.server_source()
            };
            g.command_source = Some(source);
            g.run_command(line)
        });
        self.chat_capture = outer_chat;
        self.exec = outer_exec;
        self.cmds.results = outer_results;
        self.native_command_filter = filter;
        let feedback = plain_lines
            .map(|lines| std::mem::take(&mut *lines.lock().unwrap()))
            .unwrap_or_default();
        CommandResult::new(success, feedback)
    }

    pub(super) fn native_console_source(&self) -> bool {
        self.command_source
            .as_ref()
            .is_some_and(|source| source.native_output.is_some())
    }

    /// Deliver to the command source rather than the ambient player chat capture or log.
    pub(super) fn send_command_output(
        &mut self,
        kind: fn(mod_api::Text) -> CommandFeedback,
        text: Text,
    ) {
        if let Some(output) = self
            .command_source
            .as_ref()
            .and_then(|source| source.native_output.as_ref())
        {
            output.send(kind, &text);
            return;
        }
        self.system_message(text);
    }
}
