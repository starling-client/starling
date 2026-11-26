package dev.sunriselab.parity;

import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.network.PacketProcessor;
import net.minecraft.network.protocol.game.ClientboundSetCameraPacket;
import net.minecraft.world.entity.Entity;

/** Real ClientPacketListener.handleSetCamera over a level holding one entity of each class; prints the camera entity id after each packet. */
public final class VanillaSetCameraOracle {
    static Object unsafe;
    static java.lang.reflect.Method allocate;

    static final class World extends ClientLevel {
        Map<Integer, Entity> entities;
        World() { super(null, null, null, null, 0, 0, null, false, 0L, 0); }
        @Override public Entity getEntity(int id) { return entities.get(id); }
    }

    static Field field(Class<?> owner, String name) throws Exception {
        for (Class<?> c = owner; c != null; c = c.getSuperclass()) {
            try {
                Field f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException ignored) {
            }
        }
        throw new NoSuchFieldException(name);
    }

    static <T> T make(Class<T> type) throws Exception { return type.cast(allocate.invoke(unsafe, type)); }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var u = Class.forName("sun.misc.Unsafe");
        unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        int player = 7;
        int target = 40;
        int[] packets = {target, player, target, 99, target, 99};
        try (var out = Files.newBufferedWriter(Path.of(args[0]), StandardCharsets.UTF_8)) {
            for (String line : Files.readAllLines(Path.of(args[1]))) {
                // line: label<TAB>fully qualified entity class
                String[] parts = line.split("\t");
                var world = make(World.class);
                world.entities = new HashMap<>();
                var self = make(net.minecraft.world.entity.animal.pig.Pig.class);
                self.setId(player);
                world.entities.put(player, self);
                var other = make(Class.forName(parts[1]).asSubclass(Entity.class));
                other.setId(target);
                world.entities.put(target, other);
                var camera = make(Camera.class);
                camera.setEntity(self);
                var renderer = make(GameRenderer.class);
                field(GameRenderer.class, "mainCamera").set(renderer, camera);
                var minecraft = make(Minecraft.class);
                field(Minecraft.class, "gameRenderer").set(minecraft, renderer);
                field(Minecraft.class, "packetProcessor").set(minecraft, new PacketProcessor(Thread.currentThread()));
                var listener = make(ClientPacketListener.class);
                field(ClientPacketListener.class, "minecraft").set(listener, minecraft);
                field(ClientPacketListener.class, "level").set(listener, world);
                var after = new StringJoiner(",");
                for (int id : packets) {
                    var packet = make(ClientboundSetCameraPacket.class);
                    field(ClientboundSetCameraPacket.class, "cameraId").set(packet, id);
                    listener.handleSetCamera(packet);
                    after.add(Integer.toString(camera.entity().getId()));
                }
                out.write(parts[0] + "\t" + after + "\n");
            }
        }
    }
}
