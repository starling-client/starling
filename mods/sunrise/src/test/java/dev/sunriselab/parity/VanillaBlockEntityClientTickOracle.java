package dev.sunriselab.parity;

import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.sounds.SoundSource;
import net.minecraft.util.ProblemReporter;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.storage.TagValueInput;
import net.minecraft.world.entity.EntityDimensions;
import net.minecraft.world.entity.HumanoidArm;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.SpawnerBlockEntity;
import net.minecraft.world.level.block.entity.TrialSpawnerBlockEntity;
import net.minecraft.world.level.block.entity.vault.VaultBlockEntity;
import net.minecraft.world.level.block.entity.vault.VaultSharedData;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.phys.Vec3;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.player.AbstractClientPlayer;
import net.minecraft.client.renderer.LevelEventHandler;

/**
 * Original oracle for the client ticks of spawners, trial spawners and vaults
 * (BaseSpawner.clientTick, TrialSpawner.tickClient, VaultBlockEntity.Client.tick) as
 * ClientLevel.tickBlockEntities runs them: every loaded one, each client tick, in the level's
 * ticker order, all drawing from the one level random. A case's columns arrive as chunk packets:
 * LevelChunk.replaceWithPacketData drops the column's block entities and makes those the packet
 * lists, each loading the update tag the server's copy gives, in its order (forEachBlockEntityTag, getBlockEntity(IMMEDIATE), addAndRegisterBlockEntity),
 * so their tickers join the level's list in packet order; a forgotten column's tickers go.
 * Level events 3011 to 3021 run through LevelEventHandler.levelEvent itself.
 *
 * The level is allocated without ClientLevel's constructor and answers only what these ticks ask:
 * its random, game time, players, block entities, particles and local sounds. Spawners get a
 * display entity first, as SpawnerRenderer makes one (BaseSpawner.clientTick does nothing
 * without it).
 *
 * Input lines: case JSON. Output lines: {ticks: [{events, random, spins}]} or {error}. Doubles,
 * floats and sound seeds are their raw bits, as hex strings.
 */
public final class VanillaBlockEntityClientTickOracle {
    private static Object unsafe;
    private static Method allocate;

    static <T> T allocate(Class<T> type) throws Exception {
        return type.cast(allocate.invoke(unsafe, type));
    }

    static void set(Class<?> owner, Object target, String name, Object value) throws Exception {
        Field field = owner.getDeclaredField(name);
        field.setAccessible(true);
        field.set(target, value);
    }

    static String bits(double d) { return Long.toHexString(Double.doubleToRawLongBits(d)); }

    static String bits(float f) { return Integer.toHexString(Float.floatToRawIntBits(f)); }

    /** The player these ticks look for: alive or not, a spectator or not, standing. */
    static final class CasePlayer extends AbstractClientPlayer {
        boolean dead, spectator;
        CasePlayer() { super(null, null); }
        @Override public GameType gameMode() { return spectator ? GameType.SPECTATOR : GameType.SURVIVAL; }
        @Override public HumanoidArm getMainArm() { return HumanoidArm.RIGHT; }
        @Override public boolean isSpectator() { return spectator; }
        @Override public boolean isAlive() { return !dead; }
    }

    /** The client level as these ticks see it. */
    static final class CaseLevel extends ClientLevel {
        RandomSource random;
        long gameTime;
        List<AbstractClientPlayer> casePlayers;
        Map<BlockPos, BlockEntity> loaded;
        JsonArray events;
        CaseLevel() { super(null, null, null, null, 0, 0, null, false, 0L, 0); }
        @Override public RandomSource getRandom() { return random; }
        @Override public long getGameTime() { return gameTime; }
        @Override public List<AbstractClientPlayer> players() { return casePlayers; }
        @Override public Player getPlayerByUUID(UUID uuid) {
            for (Player p : casePlayers) if (p.getUUID().equals(uuid)) return p;
            return null;
        }
        @Override public BlockEntity getBlockEntity(BlockPos pos) { return loaded.get(pos); }
        void particle(ParticleOptions options, double x, double y, double z, double vx, double vy, double vz) {
            var e = new JsonArray();
            e.add("particle");
            e.add(BuiltInRegistries.PARTICLE_TYPE.getKey(options.getType()).toString());
            for (double d : new double[] {x, y, z, vx, vy, vz}) e.add(bits(d));
            events.add(e);
        }
        @Override public void addParticle(ParticleOptions o, double x, double y, double z, double vx, double vy, double vz) { particle(o, x, y, z, vx, vy, vz); }
        @Override public void addParticle(ParticleOptions o, boolean a, boolean b, double x, double y, double z, double vx, double vy, double vz) { particle(o, x, y, z, vx, vy, vz); }
        @Override public void addAlwaysVisibleParticle(ParticleOptions o, double x, double y, double z, double vx, double vy, double vz) { particle(o, x, y, z, vx, vy, vz); }
        @Override public void addAlwaysVisibleParticle(ParticleOptions o, boolean a, double x, double y, double z, double vx, double vy, double vz) { particle(o, x, y, z, vx, vy, vz); }
        // ClientLevel.playLocalSound: the sound instance's seed is the level random's next long.
        @Override public void playLocalSound(double x, double y, double z, SoundEvent sound, SoundSource source, float volume, float pitch, boolean delay) {
            var e = new JsonArray();
            e.add("sound");
            e.add(sound.location().toString());
            for (double d : new double[] {x, y, z}) e.add(bits(d));
            e.add(bits(volume));
            e.add(bits(pitch));
            e.add(Long.toHexString(random.nextLong()));
            events.add(e);
        }
    }

    static BlockPos pos(JsonArray a) { return new BlockPos(a.get(0).getAsInt(), a.get(1).getAsInt(), a.get(2).getAsInt()); }

    /** The block state of one placed block entity. */
    static BlockState state(JsonObject e) throws Exception {
        String spec = switch (e.get("kind").getAsString()) {
            case "spawner" -> "minecraft:spawner";
            case "trial_spawner" -> "minecraft:trial_spawner[trial_spawner_state=" + e.get("state").getAsString()
                + ",ominous=" + e.get("ominous").getAsBoolean() + "]";
            case "vault" -> "minecraft:vault[facing=" + e.get("facing").getAsString() + ",ominous="
                + e.get("ominous").getAsBoolean() + ",vault_state=" + e.get("state").getAsString() + "]";
            default -> throw new IllegalArgumentException(e.get("kind").getAsString());
        };
        return BlockStateParser.parseForBlock(BuiltInRegistries.BLOCK, spec, false).blockState();
    }

    /** A block entity as the server holds it. */
    static BlockEntity make(JsonObject e, CaseLevel level, Player player) throws Exception {
        BlockPos p = pos(e.getAsJsonArray("pos"));
        BlockState state = state(e);
        BlockEntity be = ((EntityBlock) state.getBlock()).newBlockEntity(p, state);
        switch (e.get("kind").getAsString()) {
            case "spawner" -> {
                var spawner = ((SpawnerBlockEntity) be).getSpawner();
                var base = net.minecraft.world.level.BaseSpawner.class;
                set(base, spawner, "requiredPlayerRange", e.get("range").getAsInt());
                set(base, spawner, "spawnDelay", e.get("delay").getAsInt());
            }
            case "trial_spawner" -> {
                var data = ((TrialSpawnerBlockEntity) be).getTrialSpawner().getStateData();
                set(data.getClass(), data, "nextMobSpawnsAt", e.get("nextMobSpawnsAt").getAsLong());
            }
            case "vault" -> {
                var shared = ((VaultBlockEntity) be).getSharedData();
                if (e.get("display").getAsBoolean()) shared.setDisplayItem(new ItemStack(Items.DIAMOND));
                Set<UUID> connected = new HashSet<>();
                if (e.get("connected").getAsBoolean()) connected.add(player.getUUID());
                set(VaultSharedData.class, shared, "connectedPlayers", connected);
                set(VaultSharedData.class, shared, "connectedParticlesRange", e.get("particlesRange").getAsDouble());
            }
            default -> throw new IllegalArgumentException();
        }
        return be;
    }

    /**
     * The client's copy of a block entity a chunk packet lists: made for its block, then loading
     * the update tag the server's copy gives (getUpdateTag, LevelChunk.replaceWithPacketData).
     */
    static BlockEntity received(JsonObject e, CaseLevel level, Player player) throws Exception {
        var server = make(e, level, player);
        var tag = server.getUpdateTag(ParityData.lookup);
        BlockState state = server.getBlockState();
        BlockEntity be = ((EntityBlock) state.getBlock()).newBlockEntity(server.getBlockPos(), state);
        be.loadWithComponents(TagValueInput.create(ProblemReporter.DISCARDING, ParityData.lookup, tag));
        if (be instanceof SpawnerBlockEntity s) {
            // SpawnerRenderer's getOrCreateDisplayEntity: any entity will do, the tick only asks for one.
            set(net.minecraft.world.level.BaseSpawner.class, s.getSpawner(), "displayEntity",
                allocate(net.minecraft.world.entity.animal.pig.Pig.class));
        }
        return be;
    }

    static void tick(BlockEntity be, CaseLevel level) {
        BlockPos p = be.getBlockPos();
        BlockState state = be.getBlockState();
        if (be instanceof SpawnerBlockEntity s) {
            SpawnerBlockEntity.clientTick(level, p, state, s);
        } else if (be instanceof TrialSpawnerBlockEntity t) {
            t.getTrialSpawner().tickClient(level, p, state.getValue(BlockStateProperties.OMINOUS));
        } else if (be instanceof VaultBlockEntity v) {
            VaultBlockEntity.Client.tick(level, p, state, v.getClientData(), v.getSharedData());
        }
    }

    static JsonArray spin(BlockEntity be) {
        var a = new JsonArray();
        BlockPos p = be.getBlockPos();
        a.add(p.getX()); a.add(p.getY()); a.add(p.getZ());
        if (be instanceof SpawnerBlockEntity s) {
            a.add(bits(s.getSpawner().getSpin())); a.add(bits(s.getSpawner().getOSpin()));
        } else if (be instanceof TrialSpawnerBlockEntity t) {
            var data = t.getTrialSpawner().getStateData();
            a.add(bits(data.getSpin())); a.add(bits(data.getOSpin()));
        } else if (be instanceof VaultBlockEntity v) {
            a.add(bits(v.getClientData().currentSpin())); a.add(bits(v.getClientData().previousSpin()));
        }
        return a;
    }

    /** A forgotten column (ClientChunkCache.drop): its block entities and their tickers go. */
    static void forget(CaseLevel level, List<BlockEntity> tickers, long column) {
        tickers.removeIf(be -> ChunkPos.pack(be.getBlockPos()) == column);
        level.loaded.keySet().removeIf(p -> ChunkPos.pack(p) == column);
    }

    /**
     * A chunk packet for {@code column} listing the case's block entities {@code order}:
     * LevelChunk.replaceWithPacketData drops the column's block entities, then makes each listed
     * one in that order, its ticker joining the level's list.
     */
    static void send(CaseLevel level, List<BlockEntity> tickers, JsonArray entities, long column, List<Integer> order, Player player) throws Exception {
        forget(level, tickers, column);
        for (int i : order) {
            var be = received(entities.get(i).getAsJsonObject(), level, player);
            if (ChunkPos.pack(be.getBlockPos()) != column) throw new IllegalArgumentException("entity " + i + " is not in its column");
            level.loaded.put(be.getBlockPos(), be);
            tickers.add(be);
        }
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        var seedField = LegacyRandomSource.class.getDeclaredField("seed");
        seedField.setAccessible(true);
        var gson = new Gson();
        var out = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank()) continue;
            var c = JsonParser.parseString(line).getAsJsonObject();
            var result = new JsonObject();
            try {
                var level = allocate(CaseLevel.class);
                level.random = new LegacyRandomSource(c.get("seed").getAsLong());
                level.gameTime = c.get("gameTime").getAsLong();
                var pc = c.getAsJsonObject("player");
                var player = allocate(CasePlayer.class);
                var at = pc.getAsJsonArray("pos");
                var position = new Vec3(at.get(0).getAsDouble(), at.get(1).getAsDouble(), at.get(2).getAsDouble());
                var entity = net.minecraft.world.entity.Entity.class;
                set(entity, player, "position", position);
                set(entity, player, "blockPosition", BlockPos.containing(position));
                set(entity, player, "dimensions", EntityDimensions.scalable(0.6F, 1.8F));
                set(entity, player, "uuid", new UUID(0x5355_4e52_4953_45L, 1L));
                player.dead = pc.get("dead").getAsBoolean();
                player.spectator = pc.get("spectator").getAsBoolean();
                level.casePlayers = List.of(player);
                level.loaded = new HashMap<>();
                var handler = allocate(LevelEventHandler.class);
                set(LevelEventHandler.class, handler, "level", level);
                // The level's tickers, and the case's block entities with the column of each.
                List<BlockEntity> tickers = new ArrayList<>();
                var entities = c.getAsJsonArray("entities");
                List<Long> columns = new ArrayList<>();
                Map<Long, List<Integer>> listed = new HashMap<>();
                for (int i = 0; i < entities.size(); i++) {
                    long column = ChunkPos.pack(pos(entities.get(i).getAsJsonObject().getAsJsonArray("pos")));
                    if (!listed.containsKey(column)) columns.add(column);
                    listed.computeIfAbsent(column, k -> new ArrayList<>()).add(i);
                }
                // The columns arrive in the order their first block entity is listed.
                level.events = new JsonArray();
                for (long column : columns) send(level, tickers, entities, column, listed.get(column), player);
                var events = c.has("events") ? c.getAsJsonObject("events") : new JsonObject();
                var ticks = new JsonArray();
                for (int t = 0; t < c.get("ticks").getAsInt(); t++) {
                    level.events = new JsonArray();
                    // The packets handled before the tick.
                    if (events.has(Integer.toString(t))) {
                        for (var op : events.getAsJsonArray(Integer.toString(t))) {
                            var o = op.getAsJsonArray();
                            switch (o.get(0).getAsString()) {
                                case "forget" -> forget(level, tickers, ChunkPos.pack(o.get(1).getAsInt(), o.get(2).getAsInt()));
                                case "send" -> {
                                    List<Integer> order = new ArrayList<>();
                                    for (var i : o.get(3).getAsJsonArray()) order.add(i.getAsInt());
                                    send(level, tickers, entities, ChunkPos.pack(o.get(1).getAsInt(), o.get(2).getAsInt()), order, player);
                                }
                                case "event" -> handler.levelEvent(o.get(1).getAsInt(), pos(o.get(2).getAsJsonArray()), o.get(3).getAsInt());
                                default -> throw new IllegalArgumentException(o.get(0).getAsString());
                            }
                        }
                    }
                    for (var be : tickers) tick(be, level);
                    var spins = new JsonArray();
                    tickers.stream().sorted(Comparator.comparing(BlockEntity::getBlockPos)).forEach(be -> spins.add(spin(be)));
                    var row = new JsonObject();
                    row.add("events", level.events);
                    // The level random's 48-bit state after the tick: every draw, sounds' seeds too.
                    row.addProperty("random", Long.toHexString(((java.util.concurrent.atomic.AtomicLong) seedField.get(level.random)).get()));
                    row.add("spins", spins);
                    ticks.add(row);
                    level.gameTime++;
                }
                result.add("ticks", ticks);
            } catch (Throwable error) {
                var cause = error instanceof InvocationTargetException && error.getCause() != null ? error.getCause() : error;
                var sw = new java.io.StringWriter();
                cause.printStackTrace(new java.io.PrintWriter(sw));
                result.addProperty("error", sw.toString().lines().limit(6).reduce("", (a, b) -> a + b + " | "));
            }
            out.append(gson.toJson(result)).append('\n');
        }
        Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
    }
}
