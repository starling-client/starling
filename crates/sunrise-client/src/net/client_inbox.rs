//! Application-lifetime client work. Each connection owns its packet receive provider.
use mod_api::{__private::game::CallbackOwner, ClientExecutor, ClientHost};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

pub struct ClientTasks {
    // None closes the queue, including for retained producers.
    queue: Arc<Mutex<Option<VecDeque<ClientTask>>>>,
    owners: RefCell<HashMap<&'static str, CallbackOwner>>,
}

impl Default for ClientTasks {
    fn default() -> Self {
        Self {
            queue: Arc::new(Mutex::new(Some(VecDeque::new()))),
            owners: RefCell::default(),
        }
    }
}

impl ClientTasks {
    pub fn is_open(&self) -> bool {
        self.queue.lock().unwrap().is_some()
    }

    pub fn close(&self) {
        // Dropping a task can call mod destructors. Release the queue before dropping any.
        let pending = self.queue.lock().unwrap().take();
        drop(pending);
    }

    pub fn take_turn(&self) -> VecDeque<ClientTask> {
        self.queue
            .lock()
            .unwrap()
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    pub fn executor(&self, id: &'static str) -> Option<ClientExecutor> {
        if !self.is_open() {
            return None;
        }
        let owner = self
            .owners
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| CallbackOwner::for_mod(id))
            .clone();
        if !owner.is_enabled() {
            return None;
        }
        let queue = self.queue.clone();
        Some(ClientExecutor::new(move |invoke| {
            let task = ClientTask {
                owner: owner.clone(),
                invoke: Some(invoke),
            };
            if owner.is_enabled()
                && let Some(pending) = queue.lock().unwrap().as_mut()
            {
                pending.push_back(task);
            }
        }))
    }
}

impl Drop for ClientTasks {
    fn drop(&mut self) {
        self.close();
    }
}

type Task = Box<dyn FnOnce(&mut dyn ClientHost) + Send>;

/// An executor callback, with the mod that obtained the handle.
pub struct ClientTask {
    owner: CallbackOwner,
    invoke: Option<Task>,
}

impl ClientTask {
    pub fn run(mut self, host: &mut dyn ClientHost) {
        if self.owner.is_enabled()
            && let Some(invoke) = self.invoke.take()
        {
            let _ = self
                .owner
                .call("client task", "client_executor", || invoke(host));
        }
    }
}

impl Drop for ClientTask {
    fn drop(&mut self) {
        if let Some(invoke) = self.invoke.take() {
            self.owner.dispose("client task", "client_executor", invoke);
        }
    }
}
