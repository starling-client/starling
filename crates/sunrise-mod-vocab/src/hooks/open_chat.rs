//! Chat screen helper carried from upstream.
pub trait Notes {
    #[notes = r#"`ClientHost::open_chat(text)` asks to open a chat screen containing `text`. It can replace the current screen, including another chat screen, and normal screen-opening hooks can cancel or replace it. Nothing is sent. Without a local player it returns an error."#]
    fn open_chat();
}
