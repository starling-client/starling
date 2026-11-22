package dev.sunriselab.parity;

import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.Minecraft;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.model.geom.EntityModelSet;
import net.minecraft.client.model.geom.LayerDefinitions;
import net.minecraft.client.renderer.blockentity.*;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.client.resources.model.sprite.SpriteGetter;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.piston.MovingPistonBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;

/**
 * Original oracle for how far vanilla draws a block entity: the real renderer of each block's
 * block entity answers BlockEntityRenderDispatcher's questions, shouldRenderOffScreen,
 * getViewDistance and shouldRender, with the render distance BeaconRenderer reads from
 * Minecraft.getInstance().options (a stub here: the client is never started).
 *
 *   list OUTPUT          every block whose block entity has a renderer, one name per line
 *   cases INPUT OUTPUT   input lines {block, pos, camera, renderDistance};
 *                        output lines {offScreen, viewDistance, shouldRender} or {error}
 */
public final class VanillaBlockEntityReachOracle {
    private static Object unsafe;
    private static Method allocate;

    static Object allocate(Class<?> type) throws Exception {
        return allocate.invoke(unsafe, type);
    }

    static void set(Class<?> owner, Object target, String name, Object value) throws Exception {
        Field field = owner.getDeclaredField(name);
        field.setAccessible(true);
        field.set(target, value);
    }

    static BlockEntity blockEntity(Block block, BlockPos pos) {
        BlockState state = block.defaultBlockState();
        if (block instanceof MovingPistonBlock) {
            // Pistons make it themselves (MovingPistonBlock.newBlockEntity is null).
            return MovingPistonBlock.newMovingBlockEntity(pos, state, Blocks.STONE.defaultBlockState(), Direction.NORTH, true, false);
        }
        return block instanceof EntityBlock entity ? entity.newBlockEntity(pos, state) : null;
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        // Options.getEffectiveRenderDistance: renderDistance, capped by the case's server view distance.
        var minecraft = allocate(Minecraft.class);
        var options = (Options) allocate(Options.class);
        var renderDistance = (OptionInstance<?>) allocate(OptionInstance.class);
        set(Options.class, options, "renderDistance", renderDistance);
        set(Minecraft.class, minecraft, "options", options);
        set(Minecraft.class, null, "instance", minecraft);
        // The renderers' context, as VanillaBlockEntityRenderOracle builds it.
        var resources = ParityFont.resources();
        var font = VanillaTextDisplayRenderOracle.font(resources);
        var models = new EntityModelSet(LayerDefinitions.createRoots());
        SpriteGetter sprites = id -> {
            try {
                return (TextureAtlasSprite) allocate(TextureAtlasSprite.class);
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        };
        var items = new net.minecraft.client.renderer.item.ItemModelResolver(null);
        var context = new BlockEntityRendererProvider.Context(null, null, items, null, models, font, sprites, null);
        var renderers = BlockEntityRenderers.createEntityRenderers(context);
        // shouldRender(T, Vec3) erases to (BlockEntity, Vec3): called on each renderer, whatever its T.
        var shouldRender = BlockEntityRenderer.class.getMethod("shouldRender", BlockEntity.class, Vec3.class);
        var gson = new Gson();
        var out = new StringBuilder();
        if (args[0].equals("list")) {
            for (Block block : BuiltInRegistries.BLOCK) {
                var be = blockEntity(block, BlockPos.ZERO);
                if (be != null && renderers.get(be.getType()) != null) {
                    out.append(BuiltInRegistries.BLOCK.getKey(block)).append('\n');
                }
            }
            Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
            return;
        }
        for (String line : Files.readAllLines(Path.of(args[1]))) {
            if (line.isBlank()) continue;
            var c = JsonParser.parseString(line).getAsJsonObject();
            var result = new JsonObject();
            try {
                Block block = BuiltInRegistries.BLOCK.getValue(Identifier.parse(c.get("block").getAsString()));
                var p = c.getAsJsonArray("pos");
                var pos = new BlockPos(p.get(0).getAsInt(), p.get(1).getAsInt(), p.get(2).getAsInt());
                var be = blockEntity(block, pos);
                var renderer = renderers.get(be.getType());
                set(OptionInstance.class, renderDistance, "value", c.get("renderDistance").getAsInt());
                // ClientPacketListener: the server's view distance, 0 when none was sent.
                options.setServerRenderDistance(c.has("serverDistance") ? c.get("serverDistance").getAsInt() : 0);
                var cam = c.getAsJsonArray("camera");
                var camera = new Vec3(cam.get(0).getAsDouble(), cam.get(1).getAsDouble(), cam.get(2).getAsDouble());
                result.addProperty("offScreen", renderer.shouldRenderOffScreen());
                result.addProperty("viewDistance", renderer.getViewDistance());
                result.addProperty("shouldRender", (Boolean) shouldRender.invoke(renderer, be, camera));
            } catch (Throwable error) {
                var cause = error instanceof InvocationTargetException && error.getCause() != null ? error.getCause() : error;
                result.addProperty("error", String.valueOf(cause));
            }
            out.append(gson.toJson(result)).append('\n');
        }
        Files.writeString(Path.of(args[2]), out.toString(), StandardCharsets.UTF_8);
    }
}
