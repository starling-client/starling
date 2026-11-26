package dev.sunriselab.parity;

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.syncher.EntityDataAccessor;
import net.minecraft.network.syncher.SynchedEntityData;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.monster.cubemob.SulfurCube;

// Original vanilla 26.3 sulfur cube server load/save/tick/sync and client MAX_FUSE/FROM_BUCKET/tickFuse oracle.
public final class VanillaSulfurCubeFuseOracle {
    static final class Server extends ServerLevel {
        Server() { super(null, null, null, null, null, null, false, 0L, java.util.List.of(), false); }
        net.minecraft.server.MinecraftServer fake;
        @Override public net.minecraft.server.MinecraftServer getServer() { return fake; }
        net.minecraft.core.RegistryAccess registries;
        @Override public net.minecraft.core.RegistryAccess registryAccess() { return registries; }
        @Override public net.minecraft.world.Difficulty getDifficulty() { return net.minecraft.world.Difficulty.NORMAL; }
        @Override public long getGameTime() { return 0L; }
        int next = 1;
        @Override public int getNextEntityId() { return next++; }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags(); }
    }

    static final class Client extends ClientLevel {
        net.minecraft.core.RegistryAccess registries;
        Client() { super(null, null, null, null, 0, 0, null, false, 0L, 0); }
        @Override public boolean isClientSide() { return true; }
        @Override public net.minecraft.core.RegistryAccess registryAccess() { return registries; }
        @Override public net.minecraft.world.Difficulty getDifficulty() { return net.minecraft.world.Difficulty.NORMAL; }
        @Override public long getGameTime() { return 0L; }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags(); }
    }

    static final String[] FUSES = {null, "-2147483648", "-5", "-1", "0", "1", "40", "40s", "40b",
        "4294967336L", "-1.5f", "1.5d", "2147483647", "\"lit\""};
    static final String[] INVULNERABLE = {null, "0b", "1b"};

    static String maxFuse(List<SynchedEntityData.DataValue<?>> values, int maxId) {
        if (values == null) return "-";
        for (var value : values) if (value.id() == maxId) return String.valueOf(value.value());
        return "-";
    }

    static net.minecraft.nbt.CompoundTag tag(String fuse, String invulnerable) throws Exception {
        var snbt = new StringBuilder("{id:\"minecraft:sulfur_cube\",Pos:[0.5d,64d,0.5d],NoAI:1b");
        if (fuse != null) snbt.append(",fuse:").append(fuse);
        if (invulnerable != null) snbt.append(",Invulnerable:").append(invulnerable);
        return net.minecraft.nbt.TagParser.parseCompoundFully(snbt.append('}').toString());
    }

    static void emitClient(PrintWriter out, String op, SulfurCube cube, EntityDataAccessor<Integer> maxFuse) {
        out.printf(Locale.ROOT, "%s -> %d %d %b %b%n",
            op, cube.getFuse(), cube.getEntityData().get(maxFuse), cube.isPrimed(), cube.fromBucket());
    }

    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        String version = net.minecraft.SharedConstants.getCurrentVersion().name();
        var u = Class.forName("sun.misc.Unsafe");
        var uf = u.getDeclaredField("theUnsafe");
        uf.setAccessible(true);
        Object unsafe = uf.get(null);
        Method allocate = u.getMethod("allocateInstance", Class.class);
        Server serverLevel = (Server) allocate.invoke(unsafe, Server.class);
        Object server = allocate.invoke(unsafe, net.minecraft.server.dedicated.DedicatedServer.class);
        Object subs = allocate.invoke(unsafe, net.minecraft.util.debug.ServerDebugSubscribers.class);
        var sf = net.minecraft.server.MinecraftServer.class.getDeclaredField("debugSubscribers");
        sf.setAccessible(true);
        sf.set(server, subs);
        serverLevel.fake = (net.minecraft.server.MinecraftServer) server;
        var repo = net.minecraft.server.packs.repository.ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(java.util.List.of("vanilla"));
        var resources = new net.minecraft.server.packs.resources.MultiPackResourceManager(net.minecraft.server.packs.PackType.SERVER_DATA, repo.openAllSelected());
        var layers = net.minecraft.server.RegistryLayer.createRegistryAccess();
        var staticTags = net.minecraft.tags.TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(net.minecraft.server.RegistryLayer.STATIC));
        var context = net.minecraft.tags.TagLoader.buildUpdatedLookups(layers.getAccessForLoading(net.minecraft.server.RegistryLayer.WORLD), staticTags);
        var registries = net.minecraft.resources.RegistryDataLoader.load(resources, context, net.minecraft.resources.RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        staticTags.forEach(net.minecraft.core.Registry.PendingTags::apply);
        serverLevel.registries = registries;

        Client clientLevel = (Client) allocate.invoke(unsafe, Client.class);
        Field isClientSide = net.minecraft.world.level.Level.class.getDeclaredField("isClientSide");
        isClientSide.setAccessible(true);
        isClientSide.setBoolean(clientLevel, true);
        clientLevel.registries = registries;

        Method tickFuse = SulfurCube.class.getDeclaredMethod("tickFuse");
        tickFuse.setAccessible(true);
        var maxField = SulfurCube.class.getDeclaredField("MAX_FUSE");
        maxField.setAccessible(true);
        var maxFuse = (EntityDataAccessor<Integer>) maxField.get(null);
        var bucketField = SulfurCube.class.getDeclaredField("FROM_BUCKET");
        bucketField.setAccessible(true);
        var fromBucket = (EntityDataAccessor<Boolean>) bucketField.get(null);
        var type = BuiltInRegistries.ENTITY_TYPE.get(net.minecraft.resources.Identifier.parse("minecraft:sulfur_cube")).get().value();

        Path serverOut = Path.of(args[0]);
        if (serverOut.getParent() != null) Files.createDirectories(serverOut.getParent());
        try (var out = new PrintWriter(Files.newBufferedWriter(serverOut))) {
            out.printf(Locale.ROOT, "# minecraft %s%n", version);
            for (String fuse : FUSES) for (String invulnerable : INVULNERABLE) {
                var cube = (SulfurCube) type.create(serverLevel, EntitySpawnReason.LOAD);
                cube.load(net.minecraft.world.level.storage.TagValueInput.create(net.minecraft.util.ProblemReporter.DISCARDING, serverLevel.registryAccess(), tag(fuse, invulnerable)));
                int loadedFuse = cube.getFuse(), loadedMax = cube.getEntityData().get(maxFuse);
                boolean invulnerableLoaded = cube.isInvulnerable(), primed = cube.isPrimed();
                var saved = net.minecraft.world.level.storage.TagValueOutput.createWithContext(net.minecraft.util.ProblemReporter.DISCARDING, serverLevel.registryAccess());
                cube.saveWithoutId(saved);
                var tag = saved.buildResult();
                String late = maxFuse(cube.getEntityData().getNonDefaultValues(), maxFuse.id());
                cube.getEntityData().packDirty();
                for (int i = 0; i < 3; i++) tickFuse.invoke(cube);
                String ticked = maxFuse(cube.getEntityData().packDirty(), maxFuse.id());
                int fuseAfter = cube.getFuse();
                cube.load(net.minecraft.world.level.storage.TagValueInput.create(net.minecraft.util.ProblemReporter.DISCARDING, serverLevel.registryAccess(), tag("-1", null)));
                String back = maxFuse(cube.getEntityData().packDirty(), maxFuse.id());
                out.printf(Locale.ROOT, "%s %s loaded %d %d %b %b saved %s %s late %s ticked %d %s back %s%n",
                    fuse == null ? "absent" : fuse, invulnerable == null ? "absent" : invulnerable,
                    loadedFuse, loadedMax, invulnerableLoaded, primed,
                    tag.contains("fuse") ? String.valueOf(tag.getIntOr("fuse", 0)) : "-",
                    tag.contains("Invulnerable") ? String.valueOf(tag.getByteOr("Invulnerable", (byte) 0)) : "-",
                    late, fuseAfter, ticked, back);
            }
        }

        if (args.length > 1) {
            Path clientOut = Path.of(args[1]);
            if (clientOut.getParent() != null) Files.createDirectories(clientOut.getParent());
            var cube = (SulfurCube) type.create(clientLevel, EntitySpawnReason.LOAD);
            if (!cube.level().isClientSide() || cube.level() instanceof ServerLevel) {
                throw new AssertionError("expected a ClientLevel sulfur cube");
            }
            try (var out = new PrintWriter(Files.newBufferedWriter(clientOut))) {
                out.printf(Locale.ROOT, "# minecraft %s%n", version);
                emitClient(out, "init", cube, maxFuse);
                for (int value : new int[]{40, 40, 1}) {
                    cube.getEntityData().assignValues(List.of(SynchedEntityData.DataValue.create(maxFuse, value)));
                    emitClient(out, "sync " + maxFuse.id() + " int " + value, cube, maxFuse);
                    tickFuse.invoke(cube);
                    emitClient(out, "tick", cube, maxFuse);
                }
                tickFuse.invoke(cube);
                emitClient(out, "tick", cube, maxFuse);
                for (int value : new int[]{0, -1, -5, Integer.MIN_VALUE, Integer.MAX_VALUE}) {
                    cube.getEntityData().assignValues(List.of(SynchedEntityData.DataValue.create(maxFuse, value)));
                    emitClient(out, "sync " + maxFuse.id() + " int " + value, cube, maxFuse);
                    tickFuse.invoke(cube);
                    emitClient(out, "tick", cube, maxFuse);
                }
                for (boolean bucket : new boolean[]{true, false}) {
                    cube.getEntityData().assignValues(List.of(SynchedEntityData.DataValue.create(fromBucket, bucket)));
                    emitClient(out, "sync " + fromBucket.id() + " bool " + bucket, cube, maxFuse);
                    tickFuse.invoke(cube);
                    emitClient(out, "tick", cube, maxFuse);
                }
                cube.getEntityData().assignValues(List.of(SynchedEntityData.DataValue.create(maxFuse, -1)));
                emitClient(out, "sync " + maxFuse.id() + " int -1", cube, maxFuse);
                tickFuse.invoke(cube);
                emitClient(out, "tick", cube, maxFuse);
            }
        }
        System.exit(0);
    }
}
