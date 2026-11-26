package dev.sunriselab.parity;

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.Camera;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.entity.EntityRenderDispatcher;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.client.renderer.extract.LevelExtractor;
import net.minecraft.client.renderer.state.level.LevelRenderState;
import net.minecraft.world.TickRateManager;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.npc.villager.Villager;
import net.minecraft.world.phys.Vec3;

/**
 * Runs the real LevelExtractor.extractVisibleEntities (the camera-entity and
 * local-player draw decision) for the local player and a villager over the
 * camera entity, the camera type and the sleeping state of both. Frustum and
 * chunk visibility are stubbed to "visible", entity extraction records the entity.
 * Prints one line per case: camera,type,player_sleeping,villager_sleeping -> drawn list.
 */
public final class VanillaCameraDrawOracle {
    static Object unsafe;
    static Method allocate;

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static Field field(Class<?> type, String name) throws Exception {
        Field f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static final class Player extends LocalPlayer {
        boolean sleeping;

        Player() {
            super(null, null, null, null, null, net.minecraft.world.entity.player.Input.EMPTY, false, null, null);
        }

        @Override public boolean isSleeping() { return sleeping; }
    }

    static final class Vill extends Villager {
        boolean sleeping;

        Vill() {
            super(null, null);
        }

        @Override public boolean isSleeping() { return sleeping; }
    }

    static final class Level extends ClientLevel {
        List<Entity> entities = new ArrayList<>();
        TickRateManager rate = new TickRateManager();

        Level() {
            super(null, null, null, null, 0, 0, null, false, 0L, 0);
        }

        @Override public Iterable<Entity> entitiesForRendering() { return entities; }
        @Override public TickRateManager tickRateManager() { return rate; }
    }

    static final class Dispatcher extends EntityRenderDispatcher {
        List<Entity> seen = new ArrayList<>();

        Dispatcher() {
            super(null, null, null, null, null, null, null, null, null, null, null);
        }

        @Override public <E extends Entity> EntityRenderState extractEntity(E entity, float partialTicks) {
            seen.add(entity);
            return new EntityRenderState();
        }
    }

    static final class Extractor extends LevelExtractor {
        Extractor() {
            super(null, null, null);
        }

        @Override public boolean isEntityVisible(Entity e, net.minecraft.client.renderer.culling.Frustum f, double x, double y, double z, float p, long c) {
            return true;
        }
    }

    static final class Cam extends Camera { }

    static boolean has(List<Entity> list, Entity e) {
        for (var x : list) {
            if (x == e) {
                return true;
            }
        }
        return false;
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        var shell = make(Minecraft.class);
        field(Minecraft.class, "fixerUpper").set(shell, net.minecraft.util.datafix.DataFixers.getDataFixer());
        field(Minecraft.class, "instance").set(null, shell);
        var options = new Options(shell, Files.createTempDirectory("opts").toFile());
        field(Minecraft.class, "options").set(shell, options);
        var level = make(Level.class);
        level.rate = new TickRateManager();
        level.entities = new ArrayList<>();
        field(Minecraft.class, "level").set(shell, level);
        var dispatcher = make(Dispatcher.class);
        dispatcher.seen = new ArrayList<>();
        var renderer = make(LevelRenderer.class);
        field(LevelRenderer.class, "entityRenderDispatcher").set(renderer, dispatcher);
        var extractor = make(Extractor.class);
        field(LevelExtractor.class, "minecraft").set(extractor, shell);
        field(LevelExtractor.class, "level").set(extractor, level);
        field(LevelExtractor.class, "levelRenderer").set(extractor, renderer);
        var visible = LevelExtractor.class.getDeclaredMethod("extractVisibleEntities", Camera.class,
            net.minecraft.client.renderer.culling.Frustum.class, DeltaTracker.class, LevelRenderState.class);
        visible.setAccessible(true);
        var position = field(Entity.class, "position");
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0]), StandardCharsets.UTF_8))) {
            for (int camera = 0; camera < 2; camera++) {
                for (int type = 0; type < 3; type++) {
                    for (int ps = 0; ps < 2; ps++) {
                        for (int vs = 0; vs < 2; vs++) {
                            var player = make(Player.class);
                            player.sleeping = ps == 1;
                            position.set(player, new Vec3(0, 64, 0));
                            var villager = make(Vill.class);
                            villager.sleeping = vs == 1;
                            position.set(villager, new Vec3(5, 64, 0));
                            level.entities.clear();
                            level.entities.add(player);
                            level.entities.add(villager);
                            dispatcher.seen.clear();
                            var cam = make(Cam.class);
                            field(Camera.class, "entity").set(cam, camera == 0 ? player : villager);
                            field(Camera.class, "detached").set(cam, type != 0);
                            field(Camera.class, "position").set(cam, new Vec3(0, 65, 0));
                            var state = new LevelRenderState();
                            visible.invoke(extractor, cam, null, DeltaTracker.ONE, state);
                            out.println((camera == 0 ? "player" : "villager") + " " + type + " " + ps + " " + vs + " "
                                + (has(dispatcher.seen, player) ? 1 : 0) + " " + (has(dispatcher.seen, villager) ? 1 : 0));
                        }
                    }
                }
            }
        }
    }
}
