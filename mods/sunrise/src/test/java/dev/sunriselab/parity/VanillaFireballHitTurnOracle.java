package dev.sunriselab.parity;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.mojang.authlib.GameProfile;
import com.mojang.authlib.services.ServicesKeySet;
import com.mojang.serialization.Lifecycle;
import io.netty.channel.ChannelHandlerContext;
import io.netty.channel.ChannelOutboundHandlerAdapter;
import io.netty.channel.ChannelPromise;
import io.netty.channel.embedded.EmbeddedChannel;
import java.lang.reflect.Field;
import java.net.Proxy;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Optional;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.BooleanSupplier;
import net.minecraft.SharedConstants;
import net.minecraft.SystemReport;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.UUIDUtil;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.game.ClientboundSoundEntityPacket;
import net.minecraft.network.protocol.game.ClientboundSoundPacket;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.Services;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.WorldStem;
import net.minecraft.server.level.ChunkMap;
import net.minecraft.server.level.ClientInformation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.level.Ticket;
import net.minecraft.server.level.TicketType;
import net.minecraft.server.level.progress.LoggingLevelLoadListener;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.server.notifications.EmptyNotificationService;
import net.minecraft.server.notifications.NotificationManager;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.permissions.PermissionSet;
import net.minecraft.server.players.NameAndId;
import net.minecraft.server.players.PlayerList;
import net.minecraft.server.players.ProfileResolver;
import net.minecraft.server.players.UserNameToIdResolver;
import net.minecraft.util.RandomSource;
import net.minecraft.util.Util;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.util.debugchart.LocalSampleLogger;
import net.minecraft.util.debugchart.SampleLogger;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.projectile.hurtingprojectile.AbstractHurtingProjectile;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.BubbleColumnBlock;
import net.minecraft.world.level.block.LiquidBlock;
import net.minecraft.world.level.dimension.LevelStem;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.levelgen.WorldDimensions;
import net.minecraft.world.level.levelgen.WorldGenSettings;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.storage.LevelDataAndDimensions;
import net.minecraft.world.level.storage.LevelStorageSource;
import net.minecraft.world.level.storage.PrimaryLevelData;
import net.minecraft.world.phys.Vec3;

// Witnesses vanilla 26.3 AbstractHurtingProjectile hit-tick rotation, near-zero normalization,
// pre-impact fluid/bubble-column/water-splash phase, repeat Breeze deflection, and saved-pitch loading.
public final class VanillaFireballHitTurnOracle {
    static final long SEED_BASE = 23880L;
    static final String OWNER_UUID = "[I;2388,0,0,1]";

    static final class Resolver implements ProfileResolver, UserNameToIdResolver {
        @Override public Optional<GameProfile> fetchByName(String name) { return Optional.empty(); }
        @Override public Optional<GameProfile> fetchById(UUID id) { return Optional.empty(); }
        @Override public void add(NameAndId nameAndId) {}
        @Override public Optional<NameAndId> get(String name) { return Optional.empty(); }
        @Override public Optional<NameAndId> get(UUID id) { return Optional.empty(); }
        @Override public void resolveOfflineUsers(boolean value) {}
        @Override public void save() {}
    }

    static String dBits(double v) {
        return String.format("%016x", Double.doubleToRawLongBits(v));
    }

    static String fBits(float v) {
        return String.format("%08x", Float.floatToRawIntBits(v));
    }

    static JsonArray vecBits(Vec3 v) {
        JsonArray a = new JsonArray();
        a.add(dBits(v.x));
        a.add(dBits(v.y));
        a.add(dBits(v.z));
        return a;
    }

    static JsonArray rotBits(float yaw, float pitch) {
        JsonArray a = new JsonArray();
        a.add(fBits(yaw));
        a.add(fBits(pitch));
        return a;
    }

    static Field field(Class<?> cls, String name) throws ReflectiveOperationException {
        Field f = cls.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static final class Server extends MinecraftServer {
        final Path output;
        final LocalSampleLogger sampleLogger = new LocalSampleLogger(4);
        boolean ran;
        ServerPlayer player;
        EmbeddedChannel channel;
        final List<String> tickSounds = new ArrayList<>();
        Field fEntityRandom;
        Field fLegacySeed;

        static Server create(Thread thread, LevelStorageSource.LevelStorageAccess access, PackRepository packs, Path output) throws Exception {
            packs.reload();
            WorldDataConfiguration config = WorldDataConfiguration.DEFAULT;
            LevelSettings settings = new LevelSettings("FireballHitTurn", GameType.SURVIVAL, LevelSettings.DifficultySettings.DEFAULT, true, config);
            WorldLoader.PackConfig packConfig = new WorldLoader.PackConfig(packs, config, false, true);
            WorldLoader.InitConfig init = new WorldLoader.InitConfig(packConfig, Commands.CommandSelection.INTEGRATED, LevelBasedPermissionSet.OWNER);
            WorldOptions options = new WorldOptions(1L, false, false);
            WorldStem stem = Util.<WorldStem>blockUntilDone(executor -> WorldLoader.load(init, context -> {
                Registry<LevelStem> noDatapackDimensions = new MappedRegistry<>(Registries.LEVEL_STEM, Lifecycle.stable()).freeze();
                WorldDimensions dimensions = context.datapackWorldRegistries().lookupOrThrow(Registries.WORLD_PRESET)
                    .getOrThrow(WorldPresets.FLAT).value().createWorldDimensions();
                WorldDimensions.Complete complete = dimensions.bake(noDatapackDimensions);
                PrimaryLevelData data = new PrimaryLevelData(settings, complete.specialWorldProperty(), complete.lifecycle());
                return new WorldLoader.DataLoadOutput<>(
                    new LevelDataAndDimensions.WorldDataAndGenSettings(data, new WorldGenSettings(options, dimensions)),
                    complete.dimensionsRegistryAccess());
            }, WorldStem::new, Util.backgroundExecutor(), executor)).get();
            return new Server(thread, access, packs, stem, output);
        }

        Server(Thread thread, LevelStorageSource.LevelStorageAccess access, PackRepository packs, WorldStem stem, Path output) {
            super(thread, access, packs, stem, Optional.of(new GameRules(WorldDataConfiguration.DEFAULT.enabledFeatures())), Proxy.NO_PROXY,
                DataFixers.getDataFixer(), new Services(null, ServicesKeySet.EMPTY, null, new Resolver(), new Resolver()),
                LoggingLevelLoadListener.forDedicatedServer(), false, new NotificationManager());
            this.output = output;
        }

        @Override protected boolean initServer() {
            this.setPlayerList(new PlayerList(this, this.registries(), this.playerDataStorage, new EmptyNotificationService()) {});
            this.loadLevel();
            return true;
        }

        @Override protected void tickServer(BooleanSupplier haveTime) {
            if (ran) return;
            ran = true;
            try {
                runWitness();
            } catch (Throwable t) {
                t.printStackTrace();
                deleteTree(tempUniverse);
                Runtime.getRuntime().halt(2);
            }
            deleteTree(tempUniverse);
            System.out.flush();
            Runtime.getRuntime().halt(0);
        }

        void cmd(String text) {
            this.getCommands().performPrefixedCommand(this.createCommandSourceStack().withSuppressedOutput(), text);
        }

        Set<Integer> entityIds(ServerLevel level) {
            Set<Integer> ids = new HashSet<>();
            for (Entity e : level.getAllEntities()) ids.add(e.getId());
            return ids;
        }

        <T extends Entity> T findNew(ServerLevel level, Set<Integer> before, Class<T> cls) {
            for (Entity e : level.getAllEntities()) {
                if (!before.contains(e.getId()) && cls.isInstance(e)) return cls.cast(e);
            }
            return null;
        }

        void seedEntity(Entity e, long seed) throws Exception {
            fEntityRandom.set(e, RandomSource.create(seed));
        }

        long legacySeed(Entity e) throws Exception {
            RandomSource r = (RandomSource) fEntityRandom.get(e);
            return ((AtomicLong) fLegacySeed.get((LegacyRandomSource) r)).get();
        }

        void drainChannel() {
            channel.runPendingTasks();
            channel.outboundMessages().clear();
        }

        void clearArena(ServerLevel level) {
            List<Entity> toDiscard = new ArrayList<>();
            for (Entity e : level.getAllEntities()) {
                if (e != player) toDiscard.add(e);
            }
            for (Entity e : toDiscard) e.discard();
            for (int x = 5; x <= 12; x++) {
                for (int z = 5; z <= 13; z++) {
                    level.setBlock(new BlockPos(x, 63, z), Blocks.STONE.defaultBlockState(), 2);
                    for (int y = 64; y <= 72; y++) {
                        level.setBlock(new BlockPos(x, y, z), Blocks.AIR.defaultBlockState(), 2);
                    }
                }
            }
            player.snapTo(2.5, 64.0, 9.0, -90.0f, 0.0f);
            player.setDeltaMovement(Vec3.ZERO);
            stepTick();
            drainChannel();
            tickSounds.clear();
        }

        AbstractHurtingProjectile summonProj(ServerLevel level, String kind, double x, double y, double z, String extraNbt, long seed) throws Exception {
            Set<Integer> before = entityIds(level);
            cmd("summon minecraft:" + kind + " " + x + " " + y + " " + z + " {" + extraNbt + "}");
            AbstractHurtingProjectile p = findNew(level, before, AbstractHurtingProjectile.class);
            if (p == null) throw new IllegalStateException("summon failed for " + kind);
            seedEntity(p, seed);
            return p;
        }

        void summonMob(String kind, double x, double y, double z, String extraNbt) {
            String nbt = "NoAI:1b,Silent:1b,NoGravity:1b" + (extraNbt.isEmpty() ? "" : "," + extraNbt);
            cmd("summon minecraft:" + kind + " " + x + " " + y + " " + z + " {" + nbt + "}");
        }

        JsonObject snapTick(AbstractHurtingProjectile p, int tick) throws Exception {
            JsonObject o = new JsonObject();
            o.addProperty("tick", tick);
            o.add("pos_bits", vecBits(p.position()));
            o.add("vel_bits", vecBits(p.getDeltaMovement()));
            o.add("rot_bits", rotBits(p.getYRot(), p.getXRot()));
            o.add("old_rot_bits", rotBits(p.yRotO, p.xRotO));
            o.addProperty("accel_bits", dBits(p.accelerationPower));
            o.addProperty("alive", p.isAlive());
            o.addProperty("in_water", p.isInWater());
            o.addProperty("rng_state", legacySeed(p));
            JsonArray snd = new JsonArray();
            for (String s : tickSounds) snd.add(s);
            o.add("sounds", snd);
            return o;
        }

        JsonObject runCase(
            String id,
            String kind,
            String scenario,
            long seed,
            AbstractHurtingProjectile p,
            int ticks
        ) throws Exception {
            JsonObject c = new JsonObject();
            c.addProperty("case", id);
            c.addProperty("kind", kind);
            c.addProperty("scenario", scenario);
            c.addProperty("seed", seed);
            c.add("loaded_rot_bits", rotBits(p.getYRot(), p.getXRot()));
            c.add("loaded_old_rot_bits", rotBits(p.yRotO, p.xRotO));
            JsonArray arr = new JsonArray();
            for (int t = 1; t <= ticks; t++) {
                tickSounds.clear();
                drainChannel();
                stepTick();
                channel.runPendingTasks();
                arr.add(snapTick(p, t));
            }
            c.add("ticks", arr);
            return c;
        }

        String motionRotAccel(double vx, double vy, double vz, float yaw, float pitch, double accel) {
            return "Motion:[" + vx + "d," + vy + "d," + vz + "d],Rotation:[" + yaw + "f," + pitch + "f],acceleration_power:" + accel + "d";
        }

        void placeFlowingWater(ServerLevel level) {
            // Stone floor at y=65; water source at (7,66,9), flowing level=1 at (8,66,9), air at (9,66,9) -> pure +X current at (8,66,9).
            for (int x = 7; x <= 9; x++) {
                level.setBlock(new BlockPos(x, 65, 9), Blocks.STONE.defaultBlockState(), 2);
            }
            level.setBlock(new BlockPos(7, 66, 9), Blocks.WATER.defaultBlockState(), 2);
            level.setBlock(new BlockPos(8, 66, 9), Blocks.WATER.defaultBlockState().setValue(LiquidBlock.LEVEL, 1), 2);
            level.setBlock(new BlockPos(9, 66, 9), Blocks.AIR.defaultBlockState(), 2);
        }

        void placeUpwardBubbleColumn(ServerLevel level) {
            // Soul sand under bubble column at (8,66,9) with water above at (8,67,9) so (8,66,9) is inside column (+0.06 Y).
            level.setBlock(new BlockPos(8, 65, 9), Blocks.SOUL_SAND.defaultBlockState(), 2);
            level.setBlock(new BlockPos(8, 66, 9), Blocks.BUBBLE_COLUMN.defaultBlockState().setValue(BubbleColumnBlock.DRAG_DOWN, false), 2);
            level.setBlock(new BlockPos(8, 67, 9), Blocks.WATER.defaultBlockState(), 2);
        }

        void runWitness() throws Exception {
            fEntityRandom = field(Entity.class, "random");
            fLegacySeed = field(LegacyRandomSource.class, "seed");

            ServerLevel level = this.overworld();
            GameProfile profile = new GameProfile(UUIDUtil.createOfflinePlayerUUID("Player"), "Player");
            CommonListenerCookie cookie = CommonListenerCookie.createInitial(profile, false);
            player = new ServerPlayer(this, level, profile, ClientInformation.createDefault());
            Connection connection = new Connection(PacketFlow.SERVERBOUND);
            channel = new EmbeddedChannel(connection);
            channel.pipeline().addFirst("capture", new ChannelOutboundHandlerAdapter() {
                @Override
                public void write(ChannelHandlerContext ctx, Object msg, ChannelPromise promise) throws Exception {
                    if (msg instanceof ClientboundSoundPacket p) {
                        tickSounds.add(p.getSound().unwrapKey().map(k -> k.identifier().toString()).orElse("direct"));
                    } else if (msg instanceof ClientboundSoundEntityPacket p) {
                        tickSounds.add(p.getSound().unwrapKey().map(k -> k.identifier().toString()).orElse("direct"));
                    }
                    super.write(ctx, msg, promise);
                }
            });
            this.getPlayerList().placeNewPlayer(connection, player, cookie);
            player.setGameMode(GameType.CREATIVE);
            player.snapTo(2.5, 64.0, 9.0, -90.0f, 0.0f);
            settle();

            cmd("gamerule spawn_mobs false");
            cmd("gamerule advance_time false");
            cmd("gamerule advance_weather false");
            cmd("gamerule mob_griefing false");
            cmd("gamerule mob_drops false");
            cmd("gamerule random_tick_speed 0");

            JsonArray cases = new JsonArray();
            long seed = SEED_BASE;

            // 1. Dry entity hits (terminal: captured before discard)
            clearArena(level);
            summonMob("pig", 8.5, 66.0, 9.0, "");
            cases.add(runCase("dry_entity_hit_axis", "small_fireball", "entity_hit", seed,
                summonProj(level, "small_fireball", 8.5, 66.25, 8.25, motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1), seed++), 1));

            clearArena(level);
            summonMob("pig", 8.5, 66.0, 9.0, "");
            cases.add(runCase("dry_entity_hit_diag", "fireball", "entity_hit", seed,
                summonProj(level, "fireball", 8.25, 66.25, 8.25, motionRotAccel(0.25, 0.125, 0.5, 45.0f, -10.0f, 0.1), seed++), 1));

            // 2. Block hits (terminal: captured before discard)
            clearArena(level);
            level.setBlock(new BlockPos(8, 66, 9), Blocks.STONE.defaultBlockState(), 2);
            cases.add(runCase("block_hit_axis", "small_fireball", "block_hit", seed,
                summonProj(level, "small_fireball", 8.5, 66.5, 8.75, motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1), seed++), 1));

            clearArena(level);
            level.setBlock(new BlockPos(8, 66, 9), Blocks.STONE.defaultBlockState(), 2);
            cases.add(runCase("block_hit_diag", "wither_skull", "block_hit", seed,
                summonProj(level, "wither_skull", 8.35, 66.4, 8.75, motionRotAccel(0.25, 0.125, 0.5, 45.0f, -10.0f, 0.1), seed++), 1));

            // 3. Misses (controls)
            clearArena(level);
            cases.add(runCase("miss_axis", "small_fireball", "miss", seed,
                summonProj(level, "small_fireball", 8.5, 66.5, 8.25, motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1), seed++), 1));

            clearArena(level);
            cases.add(runCase("miss_diag", "fireball", "miss", seed,
                summonProj(level, "fireball", 8.25, 66.5, 8.25, motionRotAccel(0.25, 0.125, 0.5, 45.0f, -10.0f, 0.1), seed++), 1));

            // 4. Own-dragon contact (survivors)
            clearArena(level);
            summonMob("pig", 8.5, 66.0, 9.0, "UUID:" + OWNER_UUID);
            cases.add(runCase("dragon_owner_axis", "dragon_fireball", "dragon_owner", seed,
                summonProj(level, "dragon_fireball", 8.5, 66.25, 8.25,
                    motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1) + ",LeftOwner:1b,Owner:" + OWNER_UUID, seed++), 1));

            clearArena(level);
            summonMob("pig", 8.5, 66.0, 9.0, "UUID:" + OWNER_UUID);
            cases.add(runCase("dragon_owner_diag", "dragon_fireball", "dragon_owner", seed,
                summonProj(level, "dragon_fireball", 8.25, 66.25, 8.25,
                    motionRotAccel(0.25, 0.125, 0.5, 45.0f, -10.0f, 0.1) + ",LeftOwner:1b,Owner:" + OWNER_UUID, seed++), 1));

            // 5. Seeded Breeze reflection (survivors, including dragon_fireball acceleration halving)
            clearArena(level);
            summonMob("breeze", 8.5, 66.0, 9.0, "");
            cases.add(runCase("small_breeze_axis", "small_fireball", "breeze_reflect", seed,
                summonProj(level, "small_fireball", 8.5, 66.5, 8.25, motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1), seed++), 1));

            clearArena(level);
            summonMob("breeze", 8.5, 66.0, 9.0, "");
            cases.add(runCase("fireball_breeze_diag", "fireball", "breeze_reflect", seed,
                summonProj(level, "fireball", 8.25, 66.5, 8.25, motionRotAccel(0.25, 0.125, 0.5, 45.0f, -10.0f, 0.1), seed++), 1));

            clearArena(level);
            summonMob("breeze", 8.5, 66.0, 9.0, "");
            cases.add(runCase("wither_skull_breeze_axis", "wither_skull", "breeze_reflect", seed,
                summonProj(level, "wither_skull", 8.5, 66.5, 8.25, motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1), seed++), 1));

            clearArena(level);
            summonMob("breeze", 8.5, 66.0, 9.0, "");
            cases.add(runCase("dragon_breeze_axis", "dragon_fireball", "breeze_reflect", seed,
                summonProj(level, "dragon_fireball", 8.5, 66.5, 8.25, motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1), seed++), 1));

            // 6. Point 1: Near-zero normalization below and at 1.0E-5F cutoff
            double belowCutoff = 1.0e-6;
            double atCutoff = (double) 1.0e-5f; // 9.999999747378752e-6
            clearArena(level);
            cases.add(runCase("near_zero_below_cutoff_miss", "small_fireball", "near_zero", seed,
                summonProj(level, "small_fireball", 8.5, 66.5, 8.5, motionRotAccel(0.0, 0.0, belowCutoff, 0.0f, 0.0f, -0.1), seed++), 1));

            clearArena(level);
            cases.add(runCase("near_zero_at_cutoff_miss", "small_fireball", "near_zero", seed,
                summonProj(level, "small_fireball", 8.5, 66.5, 8.5, motionRotAccel(0.0, 0.0, atCutoff, 0.0f, 0.0f, -0.1), seed++), 1));

            clearArena(level);
            summonMob("pig", 8.5, 66.0, 9.0, "");
            cases.add(runCase("near_zero_below_cutoff_entity_hit", "small_fireball", "near_zero", seed,
                summonProj(level, "small_fireball", 8.5, 66.5, 8.5499995, motionRotAccel(0.0, 0.0, belowCutoff, 0.0f, 0.0f, -0.1), seed++), 1));

            clearArena(level);
            summonMob("pig", 8.5, 66.0, 8.0, "");
            cases.add(runCase("near_zero_at_cutoff_entity_hit", "small_fireball", "near_zero", seed,
                summonProj(level, "small_fireball", 8.5, 66.5, 8.5, motionRotAccel(0.0, 0.0, atCutoff, 0.0f, 0.0f, -0.1), seed++), 1));

            // 7. Point 2: Pre-impact fluid phase (water state, current push, water-splash RNG after firstTick, bubble column)
            clearArena(level);
            placeFlowingWater(level);
            summonMob("breeze", 8.5, 66.0, 9.5, "");
            cases.add(runCase("breeze_wet_entry_after_first_tick", "small_fireball", "wet_breeze", seed,
                summonProj(level, "small_fireball", 8.5, 66.25, 8.05, motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1), seed++), 2));

            clearArena(level);
            placeFlowingWater(level);
            summonMob("breeze", 8.5, 66.0, 9.5, "");
            cases.add(runCase("breeze_wet_first_tick", "small_fireball", "wet_breeze", seed,
                summonProj(level, "small_fireball", 8.5, 66.25, 8.75, motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1), seed++), 1));

            clearArena(level);
            placeFlowingWater(level);
            summonMob("pig", 8.5, 66.0, 9.6, "UUID:" + OWNER_UUID);
            cases.add(runCase("dragon_owner_water_current", "dragon_fireball", "wet_dragon_owner", seed,
                summonProj(level, "dragon_fireball", 8.5, 66.1, 8.7,
                    motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1) + ",LeftOwner:1b,Owner:" + OWNER_UUID, seed++), 1));

            clearArena(level);
            placeUpwardBubbleColumn(level);
            summonMob("pig", 8.5, 66.0, 9.6, "UUID:" + OWNER_UUID);
            cases.add(runCase("dragon_owner_bubble_column", "dragon_fireball", "wet_dragon_owner", seed,
                summonProj(level, "dragon_fireball", 8.5, 66.1, 8.7,
                    motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1) + ",LeftOwner:1b,Owner:" + OWNER_UUID, seed++), 1));

            clearArena(level);
            placeUpwardBubbleColumn(level);
            summonMob("breeze", 8.5, 66.0, 9.5, "");
            cases.add(runCase("breeze_bubble_column", "small_fireball", "wet_breeze", seed,
                summonProj(level, "small_fireball", 8.5, 66.25, 8.75, motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1), seed++), 1));

            // 8. Point 3: Repeat contact with same Breeze vs different Breeze (small_fireball & dragon_fireball)
            clearArena(level);
            summonMob("breeze", 8.5, 66.0, 9.5, "");
            {
                long s = seed++;
                AbstractHurtingProjectile p = summonProj(level, "small_fireball", 8.5, 66.5, 8.8,
                    motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1), s);
                JsonObject c = new JsonObject();
                c.addProperty("case", "breeze_repeat_same");
                c.addProperty("kind", "small_fireball");
                c.addProperty("scenario", "breeze_repeat");
                c.addProperty("seed", s);
                c.add("loaded_rot_bits", rotBits(p.getYRot(), p.getXRot()));
                JsonArray arr = new JsonArray();
                tickSounds.clear(); drainChannel(); stepTick(); channel.runPendingTasks();
                arr.add(snapTick(p, 1));
                // Aim back at the same Breeze without changing lastDeflectedBy, accel, yaw/pitch, or random.
                p.setPos(8.5, 66.5, 8.8);
                p.setDeltaMovement(0.0, 0.0, 0.5);
                tickSounds.clear(); drainChannel(); stepTick(); channel.runPendingTasks();
                arr.add(snapTick(p, 2));
                c.add("ticks", arr);
                cases.add(c);
            }

            clearArena(level);
            summonMob("breeze", 8.5, 66.0, 9.5, ""); // Breeze A: box z 9.2..9.8
            summonMob("breeze", 8.5, 66.0, 8.7, ""); // Breeze B: box z 8.4..9.0
            cases.add(runCase("breeze_repeat_different", "small_fireball", "breeze_repeat", seed,
                summonProj(level, "small_fireball", 8.5, 66.5, 9.05, motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1), seed++), 2));

            clearArena(level);
            summonMob("breeze", 8.5, 66.0, 9.5, ""); // Breeze A: box z 9.2..9.8
            summonMob("breeze", 8.5, 66.0, 8.7, ""); // Breeze B: box z 8.4..9.0
            {
                long s = seed++;
                AbstractHurtingProjectile p = summonProj(level, "dragon_fireball", 8.5, 66.5, 9.05,
                    motionRotAccel(0.0, 0.0, 0.5, 0.0f, 0.0f, 0.1), s);
                JsonObject c = new JsonObject();
                c.addProperty("case", "dragon_breeze_repeat_same_then_different");
                c.addProperty("kind", "dragon_fireball");
                c.addProperty("scenario", "breeze_repeat");
                c.addProperty("seed", s);
                c.add("loaded_rot_bits", rotBits(p.getYRot(), p.getXRot()));
                JsonArray arr = new JsonArray();
                // Tick 1: hits Breeze A -> reverses, accel 0.1 -> 0.05
                tickSounds.clear(); drainChannel(); stepTick(); channel.runPendingTasks();
                arr.add(snapTick(p, 1));
                // Tick 2: aimed back at Breeze A -> no reverse, accel stays 0.05, no RNG draw, no sound
                p.setPos(8.5, 66.5, 9.05);
                p.setDeltaMovement(0.0, 0.0, 0.5);
                tickSounds.clear(); drainChannel(); stepTick(); channel.runPendingTasks();
                arr.add(snapTick(p, 2));
                // Tick 3: aimed back at Breeze B -> reverses, accel 0.05 -> 0.025, draws RNG, sound
                p.setPos(8.5, 66.5, 9.15);
                p.setDeltaMovement(0.0, 0.0, -0.5);
                tickSounds.clear(); drainChannel(); stepTick(); channel.runPendingTasks();
                arr.add(snapTick(p, 3));
                c.add("ticks", arr);
                cases.add(c);
            }

            // 9. Point 4: Saved pitch loading and hit-turn semantics
            clearArena(level);
            cases.add(runCase("saved_pitch_180_upward_miss", "small_fireball", "saved_pitch", seed,
                summonProj(level, "small_fireball", 8.5, 66.5, 8.5, motionRotAccel(0.0, 0.5, 0.0, 0.0f, 180.0f, 0.1), seed++), 1));

            clearArena(level);
            summonMob("pig", 8.5, 67.0, 8.5, "");
            cases.add(runCase("saved_pitch_180_upward_entity_hit", "small_fireball", "saved_pitch", seed,
                summonProj(level, "small_fireball", 8.5, 66.7, 8.5, motionRotAccel(0.0, 0.5, 0.0, 0.0f, 180.0f, 0.1), seed++), 1));

            clearArena(level);
            summonMob("breeze", 8.5, 67.0, 8.5, "");
            cases.add(runCase("saved_pitch_180_upward_breeze", "small_fireball", "saved_pitch", seed,
                summonProj(level, "small_fireball", 8.5, 66.7, 8.5, motionRotAccel(0.0, 0.5, 0.0, 0.0f, 180.0f, 0.1), seed++), 1));

            clearArena(level);
            cases.add(runCase("saved_pitch_neg400_upward_miss", "small_fireball", "saved_pitch", seed,
                summonProj(level, "small_fireball", 8.5, 66.5, 8.5, motionRotAccel(0.0, 0.5, 0.0, 0.0f, -400.0f, 0.1), seed++), 1));

            clearArena(level);
            cases.add(runCase("saved_huge_rotation_wind_charge", "wind_charge", "saved_pitch", seed,
                summonProj(level, "wind_charge", 8.5, 66.5, 8.5, motionRotAccel(0.6, 0.0, 0.0, 1.0e30f, Float.MAX_VALUE, 0.0), seed++), 3));

            JsonObject root = new JsonObject();
            root.addProperty("game_version", SharedConstants.getCurrentVersion().id());
            root.addProperty("java_version", System.getProperty("java.version"));
            root.add("cases", cases);

            Files.createDirectories(output.getParent());
            Gson gson = new GsonBuilder().setPrettyPrinting().serializeNulls().create();
            Files.writeString(output, gson.toJson(root) + "\n", StandardCharsets.UTF_8);
            System.out.println("Wrote " + cases.size() + " fireball hit-turn 26.3 witness cases to " + output);
        }

        void stepTick() {
            drain();
            super.tickServer(() -> false);
            drain();
        }

        void drain() {
            boolean more = true;
            while (more) {
                more = false;
                this.runAllTasks();
                for (ServerLevel level : this.getAllLevels()) {
                    while (level.getChunkSource().pollTask()) {
                        more = true;
                    }
                }
            }
        }

        void settle() {
            ServerLevel level = player.level();
            for (int dz = -1; dz <= 1; dz++) {
                for (int dx = -1; dx <= 1; dx++) {
                    level.getChunkSource().addTicket(
                        new Ticket(TicketType.DRAGON, ChunkMap.FORCED_TICKET_LEVEL),
                        new ChunkPos(dx, dz));
                    level.getChunk(dx, dz);
                }
            }
            long deadline = System.nanoTime() + 120_000_000_000L;
            for (int i = 0; System.nanoTime() < deadline; i++) {
                stepTick();
                boolean ready = true;
                for (int dz = -1; dz <= 1 && ready; dz++) {
                    for (int dx = -1; dx <= 1 && ready; dx++) {
                        ready = level.areEntitiesActuallyLoadedAndTicking(new ChunkPos(dx, dz));
                    }
                }
                if (ready && i >= 5) return;
                try { Thread.sleep(5); } catch (InterruptedException e) { throw new RuntimeException(e); }
            }
            throw new IllegalStateException("chunks did not load");
        }

        @Override public LevelBasedPermissionSet getProfilePermissions(NameAndId nameAndId) { return LevelBasedPermissionSet.OWNER; }
        @Override protected SampleLogger getTickTimeLogger() { return sampleLogger; }
        @Override public boolean isTickTimeLoggingEnabled() { return false; }
        @Override public SystemReport fillServerSystemReport(SystemReport report) { return report; }
        @Override public boolean isHardcore() { return false; }
        @Override public LevelBasedPermissionSet operatorUserPermissions() { return LevelBasedPermissionSet.GAMEMASTER; }
        @Override public PermissionSet getFunctionCompilationPermissions() { return LevelBasedPermissionSet.GAMEMASTER; }
        @Override public boolean shouldRconBroadcast() { return true; }
        @Override public boolean isDedicatedServer() { return false; }
        @Override public int getRateLimitPacketsPerSecond() { return 0; }
        @Override public int getCommandSpamThresholdSeconds() { return 0; }
        @Override public int getChatSpamThresholdSeconds() { return 0; }
        @Override public boolean useNativeTransport() { return false; }
        @Override public boolean isPublished() { return false; }
        @Override public boolean shouldInformAdmins() { return true; }
        @Override public boolean isSingleplayerOwner(NameAndId nameAndId) { return nameAndId.name().equalsIgnoreCase("Player"); }
        @Override public int getMaxPlayers() { return 8; }
    }

    static void deleteTree(Path root) {
        if (root == null || !Files.exists(root)) return;
        try (var stream = Files.walk(root)) {
            stream.sorted((a, b) -> b.compareTo(a)).forEach(p -> {
                try { Files.deleteIfExists(p); } catch (Exception ignored) {}
            });
        } catch (Exception ignored) {}
    }

    static Path tempUniverse;

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Path universe;
        if ("-".equals(args[0])) {
            universe = Files.createTempDirectory("sunrise-fireball-hit-turn-oracle-");
            tempUniverse = universe;
        } else {
            universe = Path.of(args[0]);
            deleteTree(universe.resolve("witness"));
            Files.createDirectories(universe);
        }
        Path output = Path.of(args[1]).toAbsolutePath();
        if (output.getParent() != null) Files.createDirectories(output.getParent());
        LevelStorageSource.LevelStorageAccess access = LevelStorageSource.createDefault(universe).createAccess("witness");
        PackRepository packs = ServerPacksSource.createPackRepository(access);
        MinecraftServer.spin(thread -> {
            try {
                return Server.create(thread, access, packs, output);
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        });
    }
}
