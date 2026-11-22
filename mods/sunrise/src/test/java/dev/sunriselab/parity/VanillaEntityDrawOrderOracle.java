package dev.sunriselab.parity;

import com.google.common.collect.ImmutableList;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.serialization.Codec;
import io.netty.buffer.Unpooled;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.UUID;
import net.minecraft.SharedConstants;
import net.minecraft.client.Camera;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.Minecraft;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.multiplayer.ClientCommonPacketListenerImpl;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.client.renderer.entity.EntityRenderDispatcher;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.client.renderer.extract.LevelExtractor;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.client.renderer.state.level.LevelRenderState;
import net.minecraft.core.BlockPos;
import net.minecraft.core.component.DataComponentMap;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.PacketProcessor;
import net.minecraft.network.protocol.game.ClientboundSetPassengersPacket;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.TickRateManager;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.boss.enderdragon.EndCrystal;
import net.minecraft.world.entity.boss.enderdragon.EnderDragon;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.item.PrimedTnt;
import net.minecraft.world.entity.monster.zombie.Zombie;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.entity.EntityTickList;
import net.minecraft.world.level.entity.LevelCallback;
import net.minecraft.world.level.entity.TransientEntitySectionManager;
import net.minecraft.world.phys.Vec3;

/** A headless fixture for the real client level, extraction and submission loops. */
public final class VanillaEntityDrawOrderOracle {
    private static final class HeadlessLevel extends ClientLevel {
        // Not called: the real constructor initializes chunks and the connected client's services.
        private HeadlessLevel() {
            super(null, null, null, null, 0, 0, null, false, 0, 63);
        }

        @Override public int getMinY() { return -64; }
        @Override public int getHeight() { return 384; }
    }

    private static final class Body extends EntityRenderState {
        final int id;

        Body(int id) {
            this.id = id;
        }
    }

    private static final class Recorder extends EntityRenderDispatcher {
        final List<Integer> submitted = new ArrayList<>();

        Recorder() {
            super(null, null, null, null, null, null, null, null, null, null, null);
        }

        // The scene places each body above the build limit. Bypass render-distance/frustum tests
        // and model construction, but let LevelExtractor apply the real camera-entity rules.
        @Override
        public <E extends Entity> boolean shouldRender(
            E entity, Frustum frustum, double x, double y, double z, float partialTicks
        ) {
            return true;
        }

        @Override
        public <E extends Entity> EntityRenderState extractEntity(E entity, float partialTicks) {
            return new Body(entity.getId());
        }

        @Override
        public <S extends EntityRenderState> void submit(
            S state, CameraRenderState camera, double x, double y, double z,
            PoseStack pose, SubmitNodeCollector output
        ) {
            submitted.add(((Body) state).id);
        }
    }

    private static Field field(Class<?> owner, String name) throws Exception {
        var field = owner.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }

    private static Method method(Class<?> owner, String name, Class<?>... parameters) throws Exception {
        var method = owner.getDeclaredMethod(name, parameters);
        method.setAccessible(true);
        return method;
    }

    private record Allocator(Object unsafe, Method allocate) {
        <T> T make(Class<T> type) throws Exception {
            return type.cast(allocate.invoke(unsafe, type));
        }
    }

    @SuppressWarnings("unchecked")
    private static void startEntityStorage(ClientLevel level) throws Exception {
        var callbacks = Class.forName(ClientLevel.class.getName() + "$EntityCallbacks")
            .getDeclaredConstructor(ClientLevel.class);
        callbacks.setAccessible(true);
        var storage = new TransientEntitySectionManager<>(Entity.class,
            (LevelCallback<Entity>) callbacks.newInstance(level));
        field(ClientLevel.class, "entityStorage").set(level, storage);
        field(ClientLevel.class, "players").set(level, new ArrayList<LocalPlayer>());
        field(ClientLevel.class, "dragonParts").set(level, new ArrayList<>());
        field(ClientLevel.class, "tickingEntities").set(level, new EntityTickList());
    }

    private static <T> OptionInstance<T> option(Codec<T> codec, T value) {
        return new OptionInstance<>("oracle", OptionInstance.noTooltip(), (c, v) -> c,
            null, codec, value, null);
    }

    private static LocalPlayer admitPlayer(Allocator allocator, ClientLevel level, int id) throws Exception {
        var player = allocator.make(LocalPlayer.class);
        field(Entity.class, "level").set(player, level);
        field(Entity.class, "passengers").set(player, ImmutableList.of());
        // The following position packet moves the body to another section without re-admitting it.
        var at = new Vec3(0.5, 337.0, 0.5);
        var pos = BlockPos.containing(at);
        field(Entity.class, "position").set(player, at);
        field(Entity.class, "blockPosition").set(player, pos);
        field(Entity.class, "chunkPosition").set(player, ChunkPos.containing(pos));
        player.setId(id);
        player.setUUID(new UUID(0, id));
        level.addEntity(player);
        return player;
    }

    private static void addEntity(ClientLevel level, int id, String type) {
        Entity entity = switch (type) {
            case "minecraft:zombie" -> new Zombie(EntityTypes.ZOMBIE, level);
            case "minecraft:tnt" -> new PrimedTnt(EntityTypes.TNT, level);
            case "minecraft:item" -> new ItemEntity(EntityTypes.ITEM, level);
            case "minecraft:ender_dragon" -> new EnderDragon(EntityTypes.ENDER_DRAGON, level);
            case "minecraft:end_crystal" -> new EndCrystal(EntityTypes.END_CRYSTAL, level);
            default -> throw new IllegalArgumentException(type);
        };
        entity.setId(id);
        if (entity instanceof EnderDragon dragon) {
            // The dragon's packet recreation assigns part IDs before level admission.
            int partId = id;
            for (var part : dragon.getSubEntities()) {
                part.setId(++partId);
            }
        }
        entity.setUUID(new UUID(0, id));
        entity.setPosRaw(id % 5, 321.0, 0.5);
        level.addEntity(entity);
    }

    private static void setPassengers(ClientPacketListener listener, ClientLevel level, String[] words) {
        int vehicle = Integer.parseInt(words[1]);
        int[] passengers = new int[words.length - 2];
        for (int i = 0; i < passengers.length; i++) {
            passengers[i] = Integer.parseInt(words[i + 2]);
        }
        var buffer = new FriendlyByteBuf(Unpooled.buffer());
        try {
            buffer.writeVarInt(vehicle);
            buffer.writeVarIntArray(passengers);
            ClientboundSetPassengersPacket.STREAM_CODEC.decode(buffer).handle(listener);
        } finally {
            buffer.release();
        }
        var actual = level.getEntity(vehicle).getPassengers().stream().map(Entity::getId).toList();
        if (!actual.equals(Arrays.stream(passengers).boxed().toList())) {
            throw new AssertionError("SetPassengers did not update the vehicle");
        }
    }

    private static HeadlessLevel newLevel(Allocator allocator) throws Exception {
        var level = allocator.make(HeadlessLevel.class);
        field(Level.class, "isClientSide").set(level, true);
        field(ClientLevel.class, "tickRateManager").set(level, new TickRateManager());
        startEntityStorage(level);
        return level;
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        // No world data load binds the prototype; the recorder only needs a nonempty stack.
        Items.STONE.builtInRegistryHolder().bindComponents(DataComponentMap.EMPTY);
        var unsafe = Class.forName("sun.misc.Unsafe");
        var allocator = new Allocator(field(unsafe, "theUnsafe").get(null), unsafe.getMethod("allocateInstance", Class.class));

        var recorder = new Recorder();
        var renderer = allocator.make(LevelRenderer.class);
        field(LevelRenderer.class, "entityRenderDispatcher").set(renderer, recorder);
        var submit = method(LevelRenderer.class, "submitEntities", PoseStack.class,
            LevelRenderState.class, SubmitNodeCollector.class);

        var level = newLevel(allocator);

        var minecraft = allocator.make(Minecraft.class);
        minecraft.level = level;
        field(Minecraft.class, "packetProcessor").set(minecraft, new PacketProcessor(Thread.currentThread()));
        var options = allocator.make(Options.class);
        field(Minecraft.class, "options").set(minecraft, options);
        field(Options.class, "renderDistance").set(options, option(Codec.INT, 8));
        field(Options.class, "entityDistanceScaling").set(options, option(Codec.DOUBLE, 1.0));
        field(Options.class, "chunkSectionFadeInTime").set(options, option(Codec.DOUBLE, 0.0));
        var listener = allocator.make(ClientPacketListener.class);
        field(ClientPacketListener.class, "level").set(listener, level);
        field(ClientCommonPacketListenerImpl.class, "minecraft").set(listener, minecraft);

        var extractor = allocator.make(LevelExtractor.class);
        field(LevelExtractor.class, "minecraft").set(extractor, minecraft);
        field(LevelExtractor.class, "levelRenderer").set(extractor, renderer);
        field(LevelExtractor.class, "level").set(extractor, level);
        var extract = method(LevelExtractor.class, "extractVisibleEntities", Camera.class,
            Frustum.class, DeltaTracker.class, LevelRenderState.class);
        var camera = allocator.make(Camera.class);
        var cameraEntity = field(Camera.class, "entity");
        field(Camera.class, "position").set(camera, new Vec3(0.5, 321.0, -4.5));
        field(Camera.class, "detached").set(camera, true);

        // Start vanilla at the level boundary rather than boot a connected client for login or
        // respawn. Both call ClientLevel.addEntity; same-id replacement and removals happen there.
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            String[] words = line.split(" ");
            switch (words[0]) {
                case "login", "respawn" -> {
                    boolean login = words[0].equals("login");
                    int id = login ? Integer.parseInt(words[1]) : minecraft.player.getId();
                    if (!login && words.length == 2) {
                        // handleRespawn constructs a fresh ClientLevel on a dimension change.
                        level = newLevel(allocator);
                        minecraft.level = level;
                        field(ClientPacketListener.class, "level").set(listener, level);
                        field(LevelExtractor.class, "level").set(extractor, level);
                    }
                    minecraft.player = admitPlayer(allocator, level, id);
                    cameraEntity.set(camera, minecraft.player);
                }
                case "add" -> addEntity(level, Integer.parseInt(words[1]), words[2]);
                case "beams" -> ((EndCrystal) level.getEntity(Integer.parseInt(words[1])))
                    .setBeamTarget(new net.minecraft.core.BlockPos(3, 325, 3));
                case "item" -> ((ItemEntity) level.getEntity(Integer.parseInt(words[1])))
                    .setItem(new ItemStack(Items.STONE));
                case "remove" -> {
                    for (int i = 1; i < words.length; i++) {
                        level.removeEntity(Integer.parseInt(words[i]), Entity.RemovalReason.DISCARDED);
                    }
                }
                case "position" -> minecraft.player.setPosRaw(0.5, 321.0, 0.5);
                case "passengers" -> setPassengers(listener, level, words);
                case "camera" -> cameraEntity.set(camera, level.getEntity(Integer.parseInt(words[1])));
                case "frame" -> {
                    var state = new LevelRenderState();
                    state.cameraRenderState.pos = camera.position();
                    extract.invoke(extractor, camera, null, DeltaTracker.ONE, state);
                    recorder.submitted.clear();
                    submit.invoke(renderer, new PoseStack(), state, null);
                    System.out.println("entity-draw-order: " + recorder.submitted);
                }
                default -> throw new IllegalArgumentException(line);
            }
        }
    }
}
