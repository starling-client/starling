package dev.sunriselab.parity;
import com.google.gson.JsonParser;
import java.io.PrintWriter;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.commands.arguments.EntityAnchorArgument;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.goal.DolphinJumpGoal;
import net.minecraft.world.entity.ai.targeting.TargetingConditions;
import net.minecraft.world.entity.animal.dolphin.Dolphin;
import net.minecraft.world.entity.animal.fox.Fox;
import net.minecraft.world.entity.animal.pig.Pig;
import net.minecraft.world.entity.boss.enderdragon.EnderDragon;
import net.minecraft.world.entity.boss.enderdragon.phases.DragonSittingScanningPhase;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.Vec3;

/**
 * Original oracle: the rotations four vanilla paths write, as raw float bits.
 * - fox.txt: Fox$FoxPounceGoal.tick on a fox with no target, from a given motion, pitch and jumping.
 * - dolphin.txt: DolphinJumpGoal.tick on a dolphin out of water, from a given motion and pitch.
 * - dragon.txt: DragonSittingScanningPhase.doServerTick on a dragon whose scan finds a player at a
 *   given place (the level answers getNearestPlayer with it), from a given yaw, yRotA and head.
 * - look_at.txt: Entity.lookAt(FEET, target) from a given position.
 */
public final class VanillaTurnOracle {
    static final class TargetLevel extends ServerLevel {
        TargetLevel() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        Player target;
        @Override public Player getNearestPlayer(TargetingConditions conditions, LivingEntity source, double x, double y, double z) { return target; }
    }
    static Object unsafe; static Method allocate;
    static <T> T make(Class<T> type) throws Exception { return type.cast(allocate.invoke(unsafe, type)); }
    static String bits(float v) { return String.format(Locale.ROOT, "%08x", Float.floatToRawIntBits(v)); }
    static String bits(double v) { return String.format(Locale.ROOT, "%016x", Double.doubleToRawLongBits(v)); }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        VanillaBlockUpdateOracle.AIR = Blocks.AIR.defaultBlockState();
        VanillaBlockUpdateOracle.OVERWORLD_TYPE = VanillaBlockUpdateOracle.loadOverworld();
        var allocator = Class.forName("sun.misc.Unsafe");
        var field = allocator.getDeclaredField("theUnsafe"); field.setAccessible(true);
        unsafe = field.get(null);
        allocate = allocator.getMethod("allocateInstance", Class.class);
        VanillaBlockUpdateOracle.unsafe = unsafe;
        VanillaBlockUpdateOracle.allocate = allocate;
        var world = VanillaBlockUpdateOracle.build(JsonParser.parseString("{\"cells\":[],\"seed\":0}").getAsJsonObject());
        world.entities = new ArrayList<>(); world.registries = ParityData.access;
        VanillaBlockUpdateOracle.set(Level.class, world, "damageSources", new net.minecraft.world.damagesource.DamageSources(ParityData.access));
        Path dir = Path.of(args[0]);
        Files.createDirectories(dir);
        double[][] motions = {{1, 0, 0}, {1, -0.0, 0}, {0.3, 0.5, 0}, {0.3, -0.5, 0}, {0.5, 1, 0}, {0.01, -1, 0},
            {0, 1, 0}, {0, -1, 0}, {0.1, 0.1, 0}, {0.1, -0.1, 0}, {0.2, 0.05, 0.3}, {1e-6, 0, 0}, {0, 0, 0}};
        float[] pitches = {0.0F, 30.0F, -12.5F};
        Field jumping = LivingEntity.class.getDeclaredField("jumping"); jumping.setAccessible(true);
        Class<?> pounceClass = Class.forName("net.minecraft.world.entity.animal.fox.Fox$FoxPounceGoal");
        Constructor<?> pounceNew = pounceClass.getDeclaredConstructor(Fox.class); pounceNew.setAccessible(true);
        Method pounceTick = pounceClass.getDeclaredMethod("tick"); pounceTick.setAccessible(true);
        try (var out = new PrintWriter(Files.newBufferedWriter(dir.resolve("fox.txt")))) {
            for (double[] v : motions) for (float pitch : pitches) for (boolean jump : new boolean[]{false, true}) {
                var fox = new Fox(EntityTypes.FOX, world);
                fox.setPos(2.5, 64, 2.5);
                fox.setDeltaMovement(v[0], v[1], v[2]);
                fox.setXRot(pitch);
                jumping.setBoolean(fox, jump);
                pounceTick.invoke(pounceNew.newInstance(fox));
                out.printf(Locale.ROOT, "%s %s %s %s %b %s%n", bits(v[0]), bits(v[1]), bits(v[2]), bits(pitch), jump, bits(fox.getXRot()));
            }
        }
        try (var out = new PrintWriter(Files.newBufferedWriter(dir.resolve("dolphin.txt")))) {
            for (double[] v : motions) for (float pitch : pitches) {
                var dolphin = new Dolphin(EntityTypes.DOLPHIN, world);
                dolphin.setPos(2.5, 64, 2.5);
                dolphin.setDeltaMovement(v[0], v[1], v[2]);
                dolphin.setXRot(pitch);
                new DolphinJumpGoal(dolphin, 10).tick();
                out.printf(Locale.ROOT, "%s %s %s %s %s%n", bits(v[0]), bits(v[1]), bits(v[2]), bits(pitch), bits(dolphin.getXRot()));
            }
        }
        var target = make(TargetLevel.class);
        Field position = Entity.class.getDeclaredField("position"); position.setAccessible(true);
        var player = make(ServerPlayer.class);
        target.target = player;
        double[][] places = {{4, 70, -3}, {0, 70, 30}, {-35, 70, 25}, {50, 70, 0}, {0.5, 70, -0.5}, {12, 70, 12}, {-1, 70, -60}};
        float[] yaws = {45.0F, 0.0F, -170.0F, 359.5F, 180.0F};
        float[] accels = {0.0F, 2.5F, -7.25F};
        double[][] heads = {{0, 72, 0}, {6, 72, -4}};
        try (var out = new PrintWriter(Files.newBufferedWriter(dir.resolve("dragon.txt")))) {
            for (double[] place : places) for (float yaw : yaws) for (float accel : accels) for (double[] head : heads) {
                var dragon = new EnderDragon(EntityTypes.ENDER_DRAGON, world);
                dragon.setPos(0, 70, 0);
                dragon.head.setPos(head[0], head[1], head[2]);
                dragon.setYRot(yaw);
                dragon.yRotA = accel;
                position.set(player, new Vec3(place[0], place[1], place[2]));
                new DragonSittingScanningPhase(dragon).doServerTick(target);
                out.printf(Locale.ROOT, "%s %s %s %s %s %s %s %s%n", bits(place[0]), bits(place[2]), bits(yaw), bits(accel),
                    bits(head[0]), bits(head[2]), bits(dragon.getYRot()), bits(dragon.yRotA));
            }
        }
        double[][] froms = {{10, 64, -5}, {0, 0, 0}, {0.5, 64.5, 0.5}};
        double[][] offsets = {{0, 0, 3}, {3, 0, 0}, {-3, 0, 0}, {0, 0, -3}, {0, 2, 2}, {0, -4, 4}, {1, 0.5, 2},
            {0, 5, 0}, {0, -5, 0}, {0, 0, 0}, {-2, 1, -0.0001}, {7, -3, -1}};
        try (var out = new PrintWriter(Files.newBufferedWriter(dir.resolve("look_at.txt")))) {
            for (double[] from : froms) for (double[] off : offsets) {
                var pig = new Pig(EntityTypes.PIG, world);
                pig.setPos(from[0], from[1], from[2]);
                pig.lookAt(EntityAnchorArgument.Anchor.FEET, new Vec3(from[0] + off[0], from[1] + off[1], from[2] + off[2]));
                out.printf(Locale.ROOT, "%s %s %s %s %s %s %s %s%n", bits(from[0]), bits(from[1]), bits(from[2]),
                    bits(off[0]), bits(off[1]), bits(off[2]), bits(pig.getYRot()), bits(pig.getXRot()));
            }
        }
    }
}
