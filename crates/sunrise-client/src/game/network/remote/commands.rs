use super::*;

impl Game<'_> {
    /// ClientboundCommandSuggestionsPacket: id, range start and length, then
    /// each suggestion's text and optional tooltip.
    pub(super) fn handle_command_suggestions(&mut self, data: &[u8]) {
        let lang = self
            .remote
            .as_ref()
            .map(|r| r.lang.clone())
            .unwrap_or_default();
        let mut r = sunrise_protocol::buf::Reader::new(data);
        let decoded = (|| {
            let id = r.varint()?;
            let start = r.varint()?;
            let length = r.varint()?;
            let n = r.count(usize::MAX)?;
            let mut list = Vec::with_capacity(n.min(1000));
            for _ in 0..n {
                let text = r.string(32767)?;
                let tooltip = if r.bool()? {
                    Some(plain(&r.nbt()?, &lang))
                } else {
                    None
                };
                list.push((text, tooltip));
            }
            sunrise_protocol::Result::Ok((id, start, length, list))
        })();
        if let Ok((id, start, length, list)) = decoded {
            let range = (start.max(0) as usize, length.max(0) as usize);
            self.chat_server_suggestions(id, range, list.clone());
            self.command_block_server_suggestions(id, range, list);
        }
    }
}
