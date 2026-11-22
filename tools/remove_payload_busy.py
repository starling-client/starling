#!/usr/bin/env python3
"""One-shot rewrite removing the guest-only receiving retry path (#3640)."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1] / "crates"
CLIENT = "sunrise-client/src/"
VOCAB = "sunrise-mod-vocab/src/"
RAW = CLIENT + "raw_payloads/"


def rewrite(file, *edits):
    path = ROOT / file
    source = path.read_text()
    for old, new in edits:
        if old not in source:
            raise ValueError(f"{path}: missing {old!r}")
        source = source.replace(old, new)
    path.write_text(source)


def cut(file, start, stop):
    source = (ROOT / file).read_text()
    begin = source.index(start)
    end = source.index(stop, begin)
    rewrite(file, (source[begin:end], ""))

cut(
    VOCAB + 'payload.rs',
    '/// A cheap snapshot of the guest',
    (
        '#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]\n'
        '// exhaustive: callers distinguish'
    ),
)

rewrite(
    VOCAB + 'payload.rs',
    (
        (
            '// exhaustive: callers distinguish real callback failure from unstarted, r'
            'etryable contention.'
        ),
        '// exhaustive: native callback failure.',
    ),
    (
        (
            '    /// The callback did not start; retain these exact bytes for an ordere'
            'd retry.\n'
            '    Busy {\n'
            '        /// The original owned application buffer, never a retry copy.\n'
            '        payload: PayloadBytes,\n'
            '        /// The resolved holder at the time contention was observed.\n'
            '        holder: PayloadBusy,\n'
            '    },\n'
        ),
        '',
    ),
    (
        (
            '/// channel (`PayloadRegistrar::play_raw`) a failure, or a busy expiry, lo'
            'gs the owner and the'
        ),
        '/// channel (`PayloadRegistrar::play_raw`) a failure logs the owner and the',
    ),
    (
        (
            '/// `Busy` returns unconsumed bytes to the transport under [`PayloadHandle'
            "Error`]'s\n"
            '/// ordering and deadline contract; this callback does not itself queue a '
            'retry.\n'
        ),
        '',
    ),
)

rewrite(
    VOCAB + 'exports/payload.rs',
    (
        'PayloadBusy, ',
        '',
    ),
)

rewrite(
    VOCAB + 'payload/registrar.rs',
    (
        'A handler that fails or\n    /// stays busy past its deadline',
        'A handler that fails',
    ),
)

rewrite(
    'sunrise-mod-runtime/src/lib.rs',
    (
        'pub const MAX_GUEST_CALL_BUDGET_MILLIS: u64 = 30_000;\n',
        '',
    ),
)

cut(
    RAW + 'connection.rs',
    'pub(super) const PAYLOAD_DEADLINE',
    '// Four slots',
)

rewrite(
    RAW + 'connection.rs',
    (
        'use std::time::{Duration, Instant};\n',
        '',
    ),
    (
        '    received: Instant,\n',
        '',
    ),
    (
        '                received: Instant::now(),\n',
        '',
    ),
)

cut(
    RAW + 'incoming.rs',
    '        if let Some(holder) = self.holder.as_ref()',
    '    /// Owned transport buffers',
)

rewrite(
    RAW + 'incoming.rs',
    (
        '    busy: Option<mod_api::PayloadBusy>,\n',
        '',
    ),
    (
        '    holder: Option<Box<mod_api::PayloadBusy>>,\n',
        '',
    ),
    (
        '    /// Owned transport buffers',
        (
            '        write!(f, "payload {} refused: {:?}", self.id.0, self.error)\n'
            '    }\n'
            '}\n'
            'impl Incoming {\n'
            '    /// Owned transport buffers'
        ),
    ),
    (
        (
            'Err(Failure::new(\n'
            '                    transport,\n'
            '                    PayloadId(packet.channel),\n'
            '                    PayloadSendError::NotNegotiated,\n'
            '                ))'
        ),
        'Err(Failure { id: PayloadId(packet.channel), error: PayloadSendError::NotNegotiated })',
    ),
    (
        'Failure::new(transport, PayloadId(packet.channel), error)',
        'Failure { id: PayloadId(packet.channel), error }',
    ),
    (
        'Failure::new(transport, PayloadId(packet.channel.clone()), error)',
        'Failure { id: PayloadId(packet.channel.clone()), error }',
    ),
    (
        '                busy: None,\n',
        '',
    ),
    (
        '            busy: None,\n',
        '',
    ),
)

cut(
    RAW + 'connection/payload_queue.rs',
    'impl PayloadTurn {',
    'impl Drop for PayloadTurn {',
)

cut(
    RAW + 'connection/payload_queue.rs',
    '    pub(in crate::raw_payloads) fn defer_payload',
    '    pub(in crate::raw_payloads) fn wait_payloads',
)

cut(
    RAW + 'connection/payload_queue.rs',
    '    pub(in crate::raw_payloads) fn busy_holder',
    '    pub(super) fn clear_payloads',
)

rewrite(
    RAW + 'connection/payload_queue.rs',
    (
        '    busy: Option<mod_api::PayloadBusy>,\n',
        '',
    ),
    (
        '    deferred: bool,\n',
        '',
    ),
    (
        '            if !self.deferred {\n                queue.busy = None;\n            }\n',
        '',
    ),
    (
        '            deferred: false,\n',
        '',
    ),
    (
        '        queue.busy = None;\n',
        '',
    ),
)

cut(
    RAW + 'executor.rs',
    '        Err(mod_api::PayloadHandleError::Busy',
    '        Err(mod_api::PayloadHandleError::Failed',
)

cut(
    RAW + 'executor.rs',
    '    if let Some(holder) = incoming.expired_busy',
    '    if matches!(incoming.body',
)

rewrite(
    RAW + 'executor.rs',
    (
        (
            '    if let Some(incoming) = connection.queue_payload(incoming)\n'
            '        && let Some(incoming) = execute(incoming, &endpoint, &main, std::t'
            'ime::Instant::now())\n'
            '    {\n'
            '        connection.defer_payload(incoming);\n'
            '    }'
        ),
        (
            '    if let Some(incoming) = connection.queue_payload(incoming) {\n'
            '        execute(incoming, &endpoint, &main);\n'
            '    }'
        ),
    ),
    (
        (
            '    dispatch_pending_at(endpoint, main, std::time::Instant::now());\n'
            '}\n'
            '\n'
            'pub(super) fn dispatch_pending_at(endpoint: &Endpoint, main: &MainQueue, n'
            'ow: std::time::Instant) {\n'
        ),
        '',
    ),
    (
        (
            '        if let Some(incoming) = execute(incoming, endpoint, main, now) {\n'
            '            connection.defer_payload(incoming);\n'
            '            break;\n'
            '        }'
        ),
        '        execute(incoming, endpoint, main);',
    ),
    (
        (
            'fn execute(\n'
            '    mut incoming: Incoming,\n'
            '    endpoint: &Endpoint,\n'
            '    main: &MainQueue,\n'
            '    now: std::time::Instant,\n'
            ') -> Option<Incoming> {'
        ),
        'fn execute(mut incoming: Incoming, endpoint: &Endpoint, main: &MainQueue) {',
    ),
    (
        'return None;',
        'return;',
    ),
    (
        '    None\n}\nfn handler_failed',
        '}\nfn handler_failed',
    ),
)

cut(
    CLIENT + 'game/modding/payload_transport.rs',
    '    pub(super) fn receive_client_payload',
    '        let seal = self.transaction_open();',
)

cut(
    CLIENT + 'game/modding/payload_transport.rs',
    '            Err(mod_api::PayloadHandleError::Busy',
    '            Err(mod_api::PayloadHandleError::Failed(error)) => Err(error),',
)

rewrite(
    CLIENT + 'game/modding/payload_transport.rs',
    (
        '        let seal = self.transaction_open();',
        (
            '    pub(super) fn run_payload(&mut self, endpoint: Endpoint, incoming: Inc'
            'oming) {\n'
            '        if !incoming.epoch.active() {\n'
            '            return;\n'
            '        }\n'
            '        let seal = self.transaction_open();'
        ),
    ),
    (
        'match body {',
        'match incoming.body {',
    ),
    (
        '        true\n    }\n}',
        '    }\n}',
    ),
)

cut(
    CLIENT + 'game/network/lan/raw_inbox.rs',
    "\nimpl Game<'_> {\n    pub(super) fn receive_raw_delivery",
    '\n    }\n}\n',
)

rewrite(
    CLIENT + 'game/network/lan/raw_inbox.rs',
    (
        '    }\n}\n\n    }\n}\n',
        '    }\n}\n',
    ),
)

rewrite(
    CLIENT + 'game/network/lan/mod.rs',
    (
        '                HostEvent::RawPayload(delivery) => self.receive_raw_delivery(delivery),',
        (
            '                HostEvent::RawPayload(delivery) => {\n'
            '                    self.run_payload(delivery.endpoint, delivery.incoming)'
            ';\n'
            '                }'
        ),
    ),
)

rewrite(
    CLIENT + 'game/network/remote/raw_ingress.rs',
    (
        (
            '                Event::RawPayload(mut incoming) => {\n'
            '                    if !self.receive_client_payload(&mut incoming)? {\n'
            '                        events.push_front(Received::Event(Event::RawPayloa'
            'd(incoming), _credit));\n'
            '                        break;\n'
            '                    }\n'
            '                }'
        ),
        (
            '                Event::RawPayload(incoming) => {\n'
            '                    if let Some(remote) = self.remote.as_ref() {\n'
            '                        self.run_payload(remote.session.payload_endpoint()'
            ', *incoming);\n'
            '                    }\n'
            '                }'
        ),
    ),
)

cut(
    CLIENT + 'lan/hub/raw_inbox.rs',
    'pub(super) struct DeferredPayload',
    'impl Hub {',
)

cut(
    CLIENT + 'lan/hub/raw_inbox.rs',
    '            if let Some(held) = self.deferred_payloads',
    '            let Some(i) = self.guests',
)

cut(
    CLIENT + 'lan/hub/raw_inbox.rs',
    '    /// A refused guest call',
    '    pub(super) fn bind_raw_player',
)

rewrite(
    CLIENT + 'lan/hub.rs',
    (
        'use raw_inbox::{DeferredPayload, PacketDrain, yield_raw};',
        'use raw_inbox::{PacketDrain, yield_raw};',
    ),
    (
        '    deferred_payloads: std::collections::HashMap<Option<u32>, DeferredPayload>,\n',
        '',
    ),
    (
        '            deferred_payloads: Default::default(),\n',
        '',
    ),
    (
        'let mut events = self.begin_payload_poll();',
        'let mut events = std::mem::take(&mut self.pending_kicks);',
    ),
)
