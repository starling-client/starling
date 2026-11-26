package dev.sunriselab.parity;
import com.google.gson.JsonParser;
import java.nio.file.*;
import java.io.PrintWriter;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.nbt.*;
import net.minecraft.tags.*;
import net.minecraft.util.ProblemReporter;
import net.minecraft.world.effect.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.animal.axolotl.Axolotl;
import net.minecraft.world.entity.animal.nautilus.Nautilus;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.storage.TagValueInput;
import net.minecraft.world.level.storage.TagValueOutput;

/**
 * Original oracle: a NoAI axolotl or nautilus loaded from NBT with a signed-short `Air`, one
 * LivingEntity.baseTick up to tickEffects, a save, a reload of that save and a second tick. The
 * eye's water, the eye and feet blocks (bubble columns at either), the Water Breathing, Conduit
 * Power and Breath of the Nautilus effects and a forced lethal in-wall hurt are controlled.
 */
public final class VanillaNoAiBreatherAirOracle {
    static final class Finished extends RuntimeException {}
    interface Scene { void scene(int scene, int mask); boolean reached(); }
    static boolean water(int scene) { return scene >= 1 && scene <= 4; }
    static boolean effect(int mask, Holder<MobEffect> effect) {
        return ((mask & 1) != 0 && effect == MobEffects.WATER_BREATHING)
            || ((mask & 2) != 0 && effect == MobEffects.CONDUIT_POWER)
            || ((mask & 4) != 0 && effect == MobEffects.BREATH_OF_THE_NAUTILUS);
    }
    static final class AxolotlActor extends Axolotl implements Scene {
        int mask, scene; boolean reached;
        AxolotlActor(Level level) { super(EntityTypes.AXOLOTL, level); }
        public void scene(int scene, int mask) { this.scene = scene; this.mask = mask; }
        public boolean reached() { return reached; }
        @Override protected boolean updateFluidInteraction() { return false; }
        @Override public boolean isEyeInFluid(TagKey<Fluid> fluid) { return water(scene) && fluid == FluidTags.WATER; }
        @Override public boolean isInWall() { return scene == 5; }
        @Override public boolean hasEffect(Holder<MobEffect> e) { return effect(mask, e); }
        @Override protected void tickEffects() { reached = true; throw new Finished(); }
    }
    static final class NautilusActor extends Nautilus implements Scene {
        int mask, scene; boolean reached;
        NautilusActor(Level level) { super(EntityTypes.NAUTILUS, level); }
        public void scene(int scene, int mask) { this.scene = scene; this.mask = mask; }
        public boolean reached() { return reached; }
        @Override protected boolean updateFluidInteraction() { return false; }
        @Override public boolean isEyeInFluid(TagKey<Fluid> fluid) { return water(scene) && fluid == FluidTags.WATER; }
        @Override public boolean isInWall() { return scene == 5; }
        @Override public boolean hasEffect(Holder<MobEffect> e) { return effect(mask, e); }
        @Override protected void tickEffects() { reached = true; throw new Finished(); }
    }

    static LivingEntity actor(String species, Level world) {
        return species.equals("axolotl") ? new AxolotlActor(world) : new NautilusActor(world);
    }

    /** Ticks `actor` once (to tickEffects) and returns its saved `Air` and `Health`. */
    static CompoundTag tick(LivingEntity actor, int scene, int mask) {
        ((Scene) actor).scene(scene, mask);
        try { actor.baseTick(); } catch (Finished done) {}
        if (!((Scene) actor).reached()) throw new AssertionError("breathing phase not reached");
        var out = TagValueOutput.createWithContext(ProblemReporter.DISCARDING, ParityData.access);
        actor.saveWithoutId(out);
        return out.buildResult();
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        VanillaBlockUpdateOracle.AIR = Blocks.AIR.defaultBlockState();
        VanillaBlockUpdateOracle.OVERWORLD_TYPE = VanillaBlockUpdateOracle.loadOverworld();
        var allocator = Class.forName("sun.misc.Unsafe");
        var field = allocator.getDeclaredField("theUnsafe"); field.setAccessible(true);
        VanillaBlockUpdateOracle.unsafe = field.get(null);
        VanillaBlockUpdateOracle.allocate = allocator.getMethod("allocateInstance", Class.class);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (String species : new String[]{"axolotl", "nautilus"}) {
                int max = species.equals("axolotl") ? 6000 : 300;
                for (int scene = 0; scene < 6; scene++) for (int mask = 0; mask < 8; mask++)
                for (int air : new int[]{10, max - 6, max - 1, max, max + 1, 32767}) {
                    if (scene == 5 && mask != 0) continue;
                    var world = VanillaBlockUpdateOracle.build(JsonParser.parseString("{\"cells\":[],\"seed\":0}").getAsJsonObject());
                    world.entities = new ArrayList<>(); world.registries = ParityData.access;
                    VanillaBlockUpdateOracle.set(Level.class, world, "damageSources", new net.minecraft.world.damagesource.DamageSources(ParityData.access));
                    boolean bubbleEye = scene == 2 || scene == 3, bubbleFeet = scene == 2 || scene == 4;
                    world.cells.put(new BlockPos(2, 65, 2), (bubbleEye ? Blocks.BUBBLE_COLUMN : water(scene) ? Blocks.WATER : scene == 5 ? Blocks.STONE : Blocks.AIR).defaultBlockState());
                    world.cells.put(new BlockPos(2, 64, 2), (bubbleFeet ? Blocks.BUBBLE_COLUMN : water(scene) ? Blocks.WATER : Blocks.AIR).defaultBlockState());
                    var probe = actor(species, world);
                    // The eye in block 65, the feet in block 64.
                    double y = 65.05 - probe.getEyeHeight();
                    var tag = new CompoundTag();
                    tag.putString("id", "minecraft:" + species);
                    var pos = new ListTag(); pos.add(DoubleTag.valueOf(2.5)); pos.add(DoubleTag.valueOf(y)); pos.add(DoubleTag.valueOf(2.5));
                    tag.put("Pos", pos);
                    tag.putByte("NoAI", (byte) 1);
                    tag.putShort("Air", (short) air);
                    if (scene == 5) tag.putFloat("Health", 1.0F);
                    var first = actor(species, world);
                    first.load(TagValueInput.create(ProblemReporter.DISCARDING, ParityData.access, tag));
                    int loaded = first.getAirSupply();
                    var saved = tick(first, scene, mask);
                    int ticked = first.getAirSupply();
                    var second = actor(species, world);
                    second.load(TagValueInput.create(ProblemReporter.DISCARDING, ParityData.access, saved));
                    int reloaded = second.getAirSupply();
                    var again = tick(second, scene, mask);
                    out.printf(Locale.ROOT, "%s %d %d %d %016x %d %d %d %d %d %08x%n", species, scene, mask, air,
                        Double.doubleToRawLongBits(y), loaded, ticked, saved.getShortOr("Air", (short) -9999),
                        reloaded, again.getShortOr("Air", (short) -9999), Float.floatToIntBits(first.getHealth()));
                }
            }
        }
    }
}
