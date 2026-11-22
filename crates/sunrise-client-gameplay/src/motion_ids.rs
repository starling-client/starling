/// `tag` with a block position packed into its low 32 bits (`motion_block_entity`).
pub fn motion_at(tag: u64, pos: [i32; 3]) -> u64 {
    let [x, y, z] = pos.map(|c| c as u32);
    tag | u64::from((x & 0x7FF) | (z & 0x7FF) << 11 | (y & 0x3FF) << 22)
}
