package dev.sunriselab.parity;

import com.mojang.blaze3d.vertex.PoseStack;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.client.Minecraft;
import net.minecraft.client.model.EntityModel;
import net.minecraft.client.multiplayer.ClientChunkCache;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.entity.EntityRenderDispatcher;
import net.minecraft.client.renderer.entity.EntityRenderer;
import net.minecraft.client.renderer.entity.LivingEntityRenderer;
import net.minecraft.client.renderer.entity.MagmaCubeRenderer;
import net.minecraft.client.renderer.entity.state.LivingEntityRenderState;
import net.minecraft.client.renderer.item.ItemModelResolver;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.Difficulty;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.animal.pig.Pig;
import net.minecraft.world.entity.monster.cubemob.MagmaCube;
import net.minecraft.world.entity.monster.zombie.Zombie;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.chunk.PalettedContainerFactory;

/** Extract a real mob's living render state and run the renderer transforms without a GPU. */
public final class VanillaLivingUpsideDownOracle {
    private static final class Renderer extends LivingEntityRenderer<LivingEntity, LivingEntityRenderState, EntityModel<LivingEntityRenderState>> {
        Renderer() { super(null, null, 0.0F); }
        @Override public Identifier getTextureLocation(LivingEntityRenderState state) { return null; }
        @Override public LivingEntityRenderState createRenderState() { return new LivingEntityRenderState(); }
    }

    private static Field field(Class<?> owner, String name) throws Exception {
        var field = owner.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }

    private static <T> T allocate(Class<T> type) throws Exception {
        var unsafe = Class.forName("sun.misc.Unsafe");
        return type.cast(unsafe.getMethod("allocateInstance", Class.class)
            .invoke(field(unsafe, "theUnsafe").get(null), type));
    }

    private static ClientLevel level() throws Exception {
        var level = allocate(ClientLevel.class);
        var data = new ClientLevel.ClientLevelData(Difficulty.NORMAL, false, false);
        var dim = ParityData.access.lookupOrThrow(Registries.DIMENSION_TYPE)
            .getOrThrow(ResourceKey.create(Registries.DIMENSION_TYPE, Identifier.withDefaultNamespace("overworld")));
        field(Level.class, "levelData").set(level, data);
        field(Level.class, "dimensionTypeRegistration").set(level, dim);
        field(Level.class, "dimension").set(level, Level.OVERWORLD);
        field(Level.class, "registryAccess").set(level, ParityData.access);
        field(Level.class, "palettedContainerFactory").set(level, PalettedContainerFactory.create(ParityData.access));
        field(ClientLevel.class, "chunkSource").set(level, new ClientChunkCache(level, 2));
        return level;
    }

    private static <T extends LivingEntity, S extends LivingEntityRenderState> String measure(
            T entity, LivingEntityRenderer<T, S, ?> renderer) throws Exception {
        S state = renderer.createRenderState();
        renderer.extractRenderState(entity, state, 1.0F);
        var rotations = LivingEntityRenderer.class.getDeclaredMethod(
            "setupRotations", LivingEntityRenderState.class, PoseStack.class, float.class, float.class);
        var scale = LivingEntityRenderer.class.getDeclaredMethod("scale", LivingEntityRenderState.class, PoseStack.class);
        rotations.setAccessible(true);
        scale.setAccessible(true);

        // The standing-mob transform in LivingEntityRenderer.submit, before submitting the model.
        var stack = new PoseStack();
        stack.scale(state.scale, state.scale, state.scale);
        rotations.invoke(renderer, state, stack, state.bodyRot, state.scale);
        stack.scale(-1.0F, -1.0F, 1.0F);
        scale.invoke(renderer, state, stack);
        stack.translate(0.0F, -1.501F, 0.0F);
        var pose = stack.last().pose();
        return String.format("%08x %08x %08x %08x%n",
            Float.floatToRawIntBits(state.boundingBoxHeight), Float.floatToRawIntBits(state.scale),
            Float.floatToRawIntBits(pose.m11()), Float.floatToRawIntBits(pose.m31()));
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var level = level();
        field(Minecraft.class, "instance").set(null, allocate(Minecraft.class));
        var basic = allocate(Renderer.class);
        var cube = allocate(MagmaCubeRenderer.class);
        var dispatcher = allocate(EntityRenderDispatcher.class);
        var resolver = new ItemModelResolver(null);
        for (var renderer : new LivingEntityRenderer<?, ?, ?>[] {basic, cube}) {
            field(EntityRenderer.class, "entityRenderDispatcher").set(renderer, dispatcher);
            field(LivingEntityRenderer.class, "itemModelResolver").set(renderer, resolver);
        }
        var out = new StringBuilder();
        for (var line : Files.readAllLines(Path.of(args[0]))) {
            var fields = line.split(" ");
            LivingEntity entity = switch (fields[0]) {
                case "zombie" -> new Zombie(EntityTypes.ZOMBIE, level);
                case "pig" -> new Pig(EntityTypes.PIG, level);
                case "magma_cube" -> new MagmaCube(EntityTypes.MAGMA_CUBE, level);
                default -> throw new IllegalArgumentException(fields[0]);
            };
            entity.setId(1);
            boolean baby = fields[1].equals("baby");
            if (entity instanceof Zombie z) z.setBaby(baby);
            if (entity instanceof Pig p) p.setBaby(baby);
            if (entity instanceof MagmaCube m) m.setSize(3, true);
            entity.getAttribute(Attributes.SCALE).setBaseValue(Double.parseDouble(fields[2]));
            entity.refreshDimensions();
            entity.setCustomName(Component.literal(fields[3]));
            out.append(entity instanceof MagmaCube m ? measure(m, cube) : measure(entity, basic));
        }
        Files.writeString(Path.of(args[1]), out);
    }
}
