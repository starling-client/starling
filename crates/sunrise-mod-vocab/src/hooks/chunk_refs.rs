pub trait Notes {
    #[notes = r#"## Loaded chunks

`level.visit_loaded_chunks(host, &mut visit)` visits the chunks the level holds now, in x-then-z
order. It loads no chunk and does not copy the loaded set. Return `ControlFlow::Break(())` to
stop or `Continue(())` to visit the next chunk."#]
    fn chunk_refs_note0();
}
