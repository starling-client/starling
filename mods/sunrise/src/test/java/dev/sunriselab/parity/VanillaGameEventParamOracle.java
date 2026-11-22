package dev.sunriselab.parity;

import java.lang.reflect.*;
import java.io.PrintWriter;
import java.nio.file.*;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.Holder;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.network.PacketProcessor;
import net.minecraft.network.protocol.game.ClientboundGameEventPacket;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.sounds.SoundSource;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.GameType;
import net.minecraft.world.phys.Vec3;

/**
 * Runs the real ClientPacketListener.handleGameEvent for CHANGE_GAME_MODE and GUARDIAN_ELDER_EFFECT on each
 * parameter (hex float bits, one per line, in the file given) and prints the game mode it hands
 * MultiPlayerGameMode.setLocalMode and whether it plays the elder guardian's curse sound.
 */
public final class VanillaGameEventParamOracle {
    static Object unsafe;
    static Method allocate;

    /** The field declared on the type or the nearest superclass that declares it. */
    static Field field(Class<?> type, String name) throws Exception {
        for (Class<?> c = type; c != null; c = c.getSuperclass()) {
            try {
                var f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException e) {
                // declared further up
            }
        }
        throw new NoSuchFieldException(type.getName() + "." + name);
    }

    static <T> T make(Class<T> type) throws Exception {
        return type.cast(allocate.invoke(unsafe, type));
    }

    static final class Client extends Minecraft {
        PacketProcessor processor;
        Client() { super(null); }
        @Override public PacketProcessor packetProcessor() { return processor; }
    }

    static final class Level extends ClientLevel {
        int curses;
        int apparitions;
        Level() { super(null, null, null, null, 0, 0, null, false, 0L, 0); }
        @Override public void addParticle(ParticleOptions particle, double x, double y, double z, double xd, double yd, double zd) {
            apparitions++;
        }
        @Override public void playSound(Entity except, double x, double y, double z, SoundEvent sound, SoundSource source, float volume, float pitch) {
            if (source != SoundSource.HOSTILE) throw new AssertionError("unexpected sound source");
            curses++;
        }
        @Override public void playSeededSound(Entity except, double x, double y, double z, Holder<SoundEvent> sound, SoundSource source, float volume, float pitch, long seed) {
            if (source != SoundSource.HOSTILE) throw new AssertionError("unexpected sound source");
            curses++;
        }
    }

    static final class Controller extends MultiPlayerGameMode {
        GameType mode;
        Controller() { super(null, null); }
        @Override public void setLocalMode(GameType mode) { this.mode = mode; }
    }

    static final class Actor extends LocalPlayer {
        Actor() { super(null, null, null, null, null, null, false, null, null); }
    }

    static final class Connection extends ClientPacketListener {
        Connection() { super(null, null, null); }
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        var client = make(Client.class);
        client.processor = new PacketProcessor(Thread.currentThread());
        var actor = make(Actor.class);
        field(Entity.class, "position").set(actor, Vec3.ZERO);
        field(Minecraft.class, "player").set(client, actor);
        var controller = make(Controller.class);
        field(Minecraft.class, "gameMode").set(client, controller);
        var level = make(Level.class);
        var connection = make(Connection.class);
        field(ClientPacketListener.class, "minecraft").set(connection, client);
        field(ClientPacketListener.class, "level").set(connection, level);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))) {
            for (String line : Files.readAllLines(Path.of(args[0]))) {
                int bits = Integer.parseUnsignedInt(line.trim(), 16);
                float param = Float.intBitsToFloat(bits);
                controller.mode = null;
                connection.handleGameEvent(new ClientboundGameEventPacket(ClientboundGameEventPacket.CHANGE_GAME_MODE, param));
                if (controller.mode == null) throw new AssertionError("no setLocalMode for " + line);
                level.curses = 0;
                level.apparitions = 0;
                connection.handleGameEvent(new ClientboundGameEventPacket(ClientboundGameEventPacket.GUARDIAN_ELDER_EFFECT, param));
                if (level.apparitions != 1 || level.curses > 1) throw new AssertionError("elder effect for " + line);
                out.println(String.format("%08x %d %d", bits, controller.mode.getId(), level.curses));
            }
        }
    }
}
