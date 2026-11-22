# Registered potions

A client mod lists the registered potions with `ClientHost::potions()` (`Registries.POTION`), beside
`enchantments()`. Each `PotionInfo` carries the potion's registry id (`minecraft:long_swiftness`), in
registry order: vanilla's 46, from `minecraft:water` to `minecraft:infested`. Content mods cannot
declare potions yet, so none of theirs appear.

The read is `ClientSetup`: call it once a client mod is set up, not every frame. The game builds the
list once; each call returns a copy.

There is no potion stack API. To show a potion, splash potion, lingering potion or tipped arrow per
potion, build the stack and give it a `minecraft:potion_contents` component whose `potion` is the
id, through `ItemStack::set_components`, as a recipe viewer does:

```rust,ignore
for potion in host.potions() {
    let mut stack = ItemStack::new(splash_potion, 1)?;
    stack.set_components(Compound::from([(
        "minecraft:potion_contents".into(),
        Tag::compound([("potion", Tag::String(potion.id.as_str().into()))]),
    )]))?;
    entries.push(stack);
}
```
