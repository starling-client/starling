package dev.sunriselab.parity;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.util.RandomSource;
import net.minecraft.util.debug.ServerDebugSubscribers;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.monster.Shulker;
import net.minecraft.world.entity.monster.breeze.Breeze;
import net.minecraft.world.entity.projectile.ShulkerBullet;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.LevelChunkSection;

// Real ShulkerBullet origin, stand-on test, and tick rotation on a recording ServerLevel.
public final class VanillaShulkerBulletOracle {
    static Method attachFace;
    static Field peek;
    static Field entityRandom;

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var allocator = Class.forName("sun.misc.Unsafe");
        var field = allocator.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        VanillaBlockUpdateOracle.unsafe = field.get(null);
        VanillaBlockUpdateOracle.allocate = allocator.getMethod("allocateInstance", Class.class);
        VanillaBlockUpdateOracle.AIR = Blocks.AIR.defaultBlockState();
        VanillaBlockUpdateOracle.OVERWORLD_TYPE = VanillaBlockUpdateOracle.loadOverworld();
        attachFace = Shulker.class.getDeclaredMethod("setAttachFace", Direction.class);
        attachFace.setAccessible(true);
        peek = Shulker.class.getDeclaredField("currentPeekAmount");
        peek.setAccessible(true);
        entityRandom = Entity.class.getDeclaredField("random");
        entityRandom.setAccessible(true);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            for (var line : Files.readAllLines(Path.of(args[0]))) {
                var input = JsonParser.parseString(line).getAsJsonObject();
                var world = VanillaBlockUpdateOracle.build(input);
                world.registries = ParityData.access;
                out.println(switch (input.get("mode").getAsString()) {
                    case "origin" -> origin(world, input);
                    case "stand" -> stand(world, input);
                    case "turn" -> turn(world, input);
                    default -> throw new IllegalArgumentException("Unknown mode: " + input.get("mode"));
                });
            }
        }
    }

    /** A shulker with an attach face, lid peek and scale fires at another shulker: the bullet's position. */
    static String origin(VanillaBlockUpdateOracle.World world, JsonObject input) throws Exception {
        var shulker = new Shulker(EntityTypes.SHULKER, world);
        shulker.getAttribute(Attributes.SCALE).setBaseValue(input.get("scale").getAsDouble());
        attachFace.invoke(shulker, Direction.from3DDataValue(input.get("face").getAsInt()));
        peek.setFloat(shulker, Float.intBitsToFloat(input.get("peek").getAsInt()));
        // setPos rebuilds the box (makeBoundingBox) with the peek just set.
        var at = input.getAsJsonArray("position");
        shulker.setPos(at.get(0).getAsDouble(), at.get(1).getAsDouble(), at.get(2).getAsDouble());
        var target = new Shulker(EntityTypes.SHULKER, world);
        var aim = input.getAsJsonArray("target");
        target.setPos(aim.get(0).getAsDouble(), aim.get(1).getAsDouble(), aim.get(2).getAsDouble());
        var bullet = new ShulkerBullet(world, shulker, target, shulker.getAttachFace().getAxis());
        return String.format("%016x %016x %016x", Double.doubleToRawLongBits(bullet.getX()),
            Double.doubleToRawLongBits(bullet.getY()), Double.doubleToRawLongBits(bullet.getZ()));
    }

    /** One block state at `pos` and a bullet with its feet at `y` above the block's centre. */
    static String stand(VanillaBlockUpdateOracle.World world, JsonObject input) throws Exception {
        BlockPos pos = VanillaBlockUpdateOracle.readPos(input.getAsJsonArray("pos"));
        world.cells.put(pos, BlockStateParser.parseForBlock(BuiltInRegistries.BLOCK, input.get("block").getAsString(), false).blockState());
        var bullet = new ShulkerBullet(EntityTypes.SHULKER_BULLET, world);
        bullet.setPos(pos.getX() + 0.5, input.get("y").getAsDouble(), pos.getZ() + 0.5);
        return Boolean.toString(world.loadedAndEntityCanStandOn(pos, bullet));
    }

    // Tick a ShulkerBullet in open air (or into a Breeze) and record exact yRot/xRot/yRotO/xRotO bits.
    static String turn(VanillaBlockUpdateOracle.World world, JsonObject input) throws Exception {
        VanillaBlockUpdateOracle.set(ChunkAccess.class, world.chunk, "levelHeightAccessor", world);
        VanillaBlockUpdateOracle.set(ChunkAccess.class, world.chunk, "sections", new LevelChunkSection[0]);
        VanillaBlockUpdateOracle.set(MinecraftServer.class, world.server, "debugSubscribers", new ServerDebugSubscribers(world.server));
        var bullet = new ShulkerBullet(EntityTypes.SHULKER_BULLET, world);
        bullet.setPos(0.5, 110.0, 0.5);
        bullet.setNoGravity(input.get("no_gravity").getAsBoolean());
        var vel = input.getAsJsonArray("velocity");
        bullet.setDeltaMovement(
            Double.longBitsToDouble(Long.parseUnsignedLong(vel.get(0).getAsString(), 16)),
            Double.longBitsToDouble(Long.parseUnsignedLong(vel.get(1).getAsString(), 16)),
            Double.longBitsToDouble(Long.parseUnsignedLong(vel.get(2).getAsString(), 16)));
        bullet.setYRot(Float.intBitsToFloat(Integer.parseUnsignedInt(input.get("yaw").getAsString(), 16)));
        bullet.setXRot(Float.intBitsToFloat(Integer.parseUnsignedInt(input.get("pitch").getAsString(), 16)));
        bullet.setOldPosAndRot();
        if (input.has("deflect_seed") && !input.get("deflect_seed").isJsonNull()) {
            ((RandomSource) entityRandom.get(bullet)).setSeed(input.get("deflect_seed").getAsLong());
            var at = input.getAsJsonArray("breeze");
            var breeze = new Breeze(EntityTypes.BREEZE, world);
            breeze.setPos(
                Double.longBitsToDouble(Long.parseUnsignedLong(at.get(0).getAsString(), 16)),
                Double.longBitsToDouble(Long.parseUnsignedLong(at.get(1).getAsString(), 16)),
                Double.longBitsToDouble(Long.parseUnsignedLong(at.get(2).getAsString(), 16)));
            world.entities = new ArrayList<>();
            world.entities.add(breeze);
        }
        int ticks = input.get("ticks").getAsInt();
        var steps = new StringBuilder();
        for (int i = 0; i < ticks; i++) {
            bullet.commonTick();
            bullet.tick();
            if (i > 0) steps.append(',');
            steps.append(String.format("%08x:%08x:%08x:%08x",
                Float.floatToRawIntBits(bullet.getYRot()),
                Float.floatToRawIntBits(bullet.getXRot()),
                Float.floatToRawIntBits(bullet.yRotO),
                Float.floatToRawIntBits(bullet.xRotO)));
        }
        return steps.toString();
    }
}
