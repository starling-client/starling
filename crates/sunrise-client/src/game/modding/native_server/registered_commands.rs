//! The mods' commands as the session holds them (`register_commands`). As on the reference
//! loader, the mods are asked whenever the session's command tree is built: when a world opens,
//! before its functions load, and on each `/reload`. The tree lives on `Game`, so a function
//! line naming a mod command parses with no hub; a hub, once there is one, takes that same tree.

use super::{
    BaseTree, CommandArguments, CommandExecutorSlots, Game, command_arguments, command_tree_for,
    register_command_nodes,
};
use mod_api::__private::{
    ClientCommandExecutor, CommandExecutor, RetainedCallback, RetainedFailure, mod_index,
    split_command_registration,
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

pub(in crate::game) struct RegisteredClientExecutor {
    pub(super) callback: RetainedCallback,
    call: Option<Arc<ClientCommandExecutor>>,
}

impl RegisteredClientExecutor {
    pub(in crate::game) fn new(
        owner: &'static str,
        index: usize,
        usage: &str,
        call: Arc<ClientCommandExecutor>,
    ) -> Self {
        Self {
            callback: RetainedCallback::new(owner, index, "client_command.execute", usage),
            call: Some(call),
        }
    }

    pub(super) fn execute(
        &self,
        host: &mut dyn mod_api::ClientHost,
        invocation: &mod_api::CommandInvocation,
    ) -> Result<Option<i32>, RetainedFailure> {
        let call = self.call.as_ref().expect("live client executor");
        self.callback.call(|| call(host, invocation))
    }
}

impl Drop for RegisteredClientExecutor {
    fn drop(&mut self) {
        self.callback
            .dispose("client_command.drop", self.call.take());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum PendingClientCommandKind {
    Signed,
    Unattended,
}

/// An invocation path, shared by all submissions from the same executor. Use the retained
/// registration identity rather than a parsed node, which can change on a tree refresh.
pub(in crate::game) struct ClientCommandChain {
    pub(super) executor: RetainedCallback,
    pub(super) parent: Option<Arc<Self>>,
}

pub(in crate::game) struct PendingClientCommand {
    pub(super) command: String,
    pub(super) kind: PendingClientCommandKind,
    pub(super) chain: Arc<ClientCommandChain>,
}

/// The client's own native mod commands, bound to the current client dispatcher generation.
pub(in crate::game) struct NativeClientCommandState {
    pub(in crate::game) dispatcher: sunrise_client::command::Dispatcher,
    pub(super) executors: Vec<Option<Arc<RegisteredClientExecutor>>>,
    /// Present while an executor runs; nested slash submissions inherit this path and wait
    /// for the client event loop instead of re-entering synchronously.
    pub(super) active_chain: Option<Arc<ClientCommandChain>>,
    pub(in crate::game) pending_commands: Vec<PendingClientCommand>,
    /// Present while `client_chat` listeners run; a chat message a listener submits waits for
    /// the next event-loop wave instead of re-entering the hook synchronously.
    pub(in crate::game) chat_dispatching: bool,
    pub(in crate::game) pending_chats: Vec<String>,
}

impl Default for NativeClientCommandState {
    fn default() -> Self {
        Self {
            dispatcher: sunrise_client::command::Dispatcher::new(),
            executors: Vec::new(),
            active_chain: None,
            pending_commands: Vec::new(),
            chat_dispatching: false,
            pending_chats: Vec::new(),
        }
    }
}

impl NativeClientCommandState {
    pub(in crate::game) fn defer(&mut self, command: &str, kind: PendingClientCommandKind) -> bool {
        let Some(chain) = self.active_chain.clone() else {
            return false;
        };
        self.pending_commands.push(PendingClientCommand {
            command: command.to_owned(),
            kind,
            chain,
        });
        true
    }

    pub(in crate::game) fn install(
        &mut self,
        dispatcher: sunrise_client::command::Dispatcher,
        executors: Vec<Option<RegisteredClientExecutor>>,
    ) {
        self.dispatcher = dispatcher;
        // A panicking retired callable may disable its replacement in the shared quarantine.
        self.executors = executors
            .into_iter()
            .map(|entry| entry.map(Arc::new))
            .collect();
    }
}

pub(super) struct RegisteredExecutor {
    pub(super) callback: RetainedCallback,
    pub(super) call: Arc<CommandExecutor>,
}

/// The commands the mods registered, as the session's tree holds them.
pub(in crate::game) struct NativeCommands {
    /// Each root's traversal level.
    pub(super) levels: HashMap<String, u8>,
    /// Accepted display data, before filtering roots of a replacement tree.
    tree: Value,
    arguments: CommandArguments,
    /// Executable path to registration slot; never sent to the hub.
    slots: CommandExecutorSlots,
    /// Process-local execution table indexed by the current dispatcher's parsed keys.
    pub(super) executors: Vec<RegisteredExecutor>,
}

/// The session's state of the mods' commands.
#[derive(Default)]
pub(in crate::game) struct NativeCommandState {
    /// A mod asked for its commands again (`refresh_commands`): done at the next server tick.
    pub(in crate::game) refresh: bool,
    pub(in crate::game) registered: Option<NativeCommands>,
    /// The display report of the currently installed server command tree.
    report: Value,
    /// A hub holds the report revision it was given, not the dispatcher incarnation.
    display_revision: u64,
}

impl NativeCommandState {
    /// The traversal level of a mod's root (`line`, without its slash), when registered.
    /// The parsed executable node enforces its own usage level separately.
    pub(in crate::game) fn level(&self, line: &str) -> Option<u8> {
        let name = line.split(' ').next().unwrap_or_default();
        self.report["children"].get(name)?;
        self.registered.as_ref()?.levels.get(name).copied()
    }
}

impl Game<'_> {
    /// `Commands`' constructor with `RegisterCommandsEvent`: asks the mods for their commands and
    /// builds the session's tree, vanilla's for `features` with the mods' on top. Called before
    /// functions are admitted (world open and each reload), so both function checks see them.
    pub(in crate::game) fn build_native_commands(&mut self, features: &[String]) {
        // A server hook: a client's tree is the one its server sends.
        if !self.is_logical_server() {
            return;
        }
        let mut registrations = Vec::new();
        self.native_register_commands(&mut registrations);
        let (usages, executors): (Vec<_>, Vec<_>) = registrations
            .into_iter()
            .filter_map(|registration| {
                let owner = registration.mod_id;
                let mod_index = mod_index(owner)?;
                let (command, call) = split_command_registration(registration.answer);
                let callback =
                    RetainedCallback::new(owner, mod_index, "command_executor", &command.usage);
                Some((command, RegisteredExecutor { callback, call }))
            })
            .unzip();
        let vanilla = sunrise_client::command::vanilla::session_dispatcher(
            &self.level.rule_registry,
            features,
        );
        let arguments = command_arguments();
        let mut slots = HashMap::new();
        let (tree, levels, problems) = command_tree_for(
            &usages,
            BaseTree::Merged(&vanilla),
            &arguments,
            false,
            Some(&mut slots),
        );
        for p in problems {
            log::warn!("Sunrise: a native mod's command was left out: {p}");
        }
        let state = &mut self.native_commands;
        state.registered = Some(NativeCommands {
            levels,
            tree,
            arguments,
            slots,
            executors,
        });
        state.display_revision = state.display_revision.wrapping_add(1);
        self.ui.dispatcher.0 = Arc::new(vanilla);
        self.ui.registries.selector_tokens =
            super::super::native_entity_selectors::selector_tokens();
        self.rebind_native_commands();
    }

    /// Gives the hub the session's mod commands when it lacks them; first asks the mods again
    /// when one asked (`native_refresh_commands`), and then sends every player their tree.
    pub(in crate::game) fn register_native_commands(&mut self) {
        let refresh = std::mem::take(&mut self.native_commands.refresh);
        if self.lan.is_none() {
            return;
        }
        if refresh || self.native_commands.registered.is_none() {
            self.build_native_commands(&self.level.enabled_features.clone());
        }
        self.send_native_command_report(refresh);
    }

    /// Send accepted display data, without rebuilding usages or exposing executor slots.
    /// Forced when vanilla itself changes, even if the accepted mod report did not.
    pub(in crate::game) fn send_native_command_report(&mut self, force_resend: bool) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let state = &self.native_commands;
        if state.registered.is_none() {
            if force_resend {
                self.send_placed_command_trees(false);
            }
            return;
        }
        let revision = Some(state.display_revision);
        let replacing = host.native_commands_sent.is_some();
        if host.native_commands_sent != revision {
            // The hub merges a root vanilla has into vanilla's node (`CommandTree::packet_with`).
            let tree = state.report.clone();
            host.native_commands_sent = revision;
            host.hub
                .set_selector_tokens(self.ui.registries.selector_tokens.clone());
            let changed = host.hub.set_mod_commands(0, tree);
            if !force_resend && !replacing && !changed {
                return;
            }
        } else if !force_resend {
            return;
        }
        self.send_placed_command_trees(false);
    }

    /// Restore mod nodes (and rebind their executors) after vanilla replaces the tree.
    pub(in crate::game) fn restore_native_commands(&mut self) {
        if self.rebind_native_commands() {
            let revision = &mut self.native_commands.display_revision;
            *revision = revision.wrapping_add(1);
        }
    }

    /// Rebind against the new node IDs. True if the display report changed too.
    fn rebind_native_commands(&mut self) -> bool {
        let state = &mut self.native_commands;
        let Some(commands) = &state.registered else {
            return false;
        };
        let dispatcher = Arc::make_mut(&mut self.ui.dispatcher.0);
        // A replacement can add a session tool, e.g. pregen. Never overwrite its nodes
        // or advertise a mod usage behind the tool's name. Vanilla's roots take mod usages.
        let report = without_reserved_roots(&commands.tree, |name| {
            sunrise_client::command::vanilla::is_session_tool(name)
                && dispatcher.find(name).is_some()
        });
        register_command_nodes(
            dispatcher,
            &report,
            "native_mod_command",
            &commands.arguments,
            Some(&commands.slots),
        );
        let changed = state.report != report;
        state.report = report;
        changed
    }
}

fn without_reserved_roots(report: &Value, reserved: impl Fn(&str) -> bool) -> Value {
    let mut report = report.clone();
    if let Some(roots) = report.get_mut("children").and_then(Value::as_object_mut) {
        roots.retain(|name, _| !reserved(name));
        if roots.is_empty() {
            return Value::Null;
        }
    }
    report
}
