package dev.sunriselab.parity;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonNull;
import com.google.gson.JsonObject;
import com.mojang.authlib.GameProfile;
import com.mojang.authlib.services.ServicesKeySet;
import com.mojang.serialization.Lifecycle;
import io.netty.channel.ChannelHandlerContext;
import io.netty.channel.ChannelOutboundHandlerAdapter;
import io.netty.channel.ChannelPromise;
import io.netty.channel.embedded.EmbeddedChannel;
import java.io.IOException;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
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
import java.util.function.BooleanSupplier;
import net.minecraft.SharedConstants;
import net.minecraft.SystemReport;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.SectionPos;
import net.minecraft.core.UUIDUtil;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.game.ClientboundEntityEventPacket;
import net.minecraft.network.protocol.game.ClientboundExplodePacket;
import net.minecraft.network.protocol.game.ClientboundSoundEntityPacket;
import net.minecraft.network.protocol.game.ClientboundSoundPacket;
import net.minecraft.network.syncher.EntityDataAccessor;
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
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.animal.pig.Pig;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.minecraft.world.entity.decoration.Cushion;
import net.minecraft.world.entity.decoration.ItemFrame;
import net.minecraft.world.entity.decoration.LeashFenceKnotEntity;
import net.minecraft.world.entity.item.FallingBlockEntity;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.projectile.Projectile;
import net.minecraft.world.entity.projectile.arrow.AbstractArrow;
import net.minecraft.world.entity.projectile.arrow.ThrownTrident;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.dimension.LevelStem;
import net.minecraft.world.level.gameevent.BlockPositionSource;
import net.minecraft.world.level.gameevent.GameEvent;
import net.minecraft.world.level.gameevent.GameEventListener;
import net.minecraft.world.level.gameevent.PositionSource;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.WorldDimensions;
import net.minecraft.world.level.levelgen.WorldGenSettings;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.storage.LevelDataAndDimensions;
import net.minecraft.world.level.storage.LevelStorageSource;
import net.minecraft.world.level.storage.PrimaryLevelData;
import net.minecraft.world.phys.Vec3;

// Runs every projectile kind against every target in a real vanilla 26.3 ServerLevel.
public final class VanillaProjectileTargetsOracle {
    static final int TICKS = 8;
    static final long PROJECTILE_SEED = 12345L;
    static final long LEVEL_SEED = 67890L;

    static final String[] PROJECTILES = {
        "arrow", "spectral_arrow", "trident", "snowball", "egg", "ender_pearl",
        "wind_charge", "small_fireball", "fireball"
    };

    static final String[] OWNED_PROJECTILES = {
        "arrow_creative", "trident_owned", "small_fireball_owned"
    };

    static final String[] TARGETS = {
        "armor_stand", "armor_stand_equipped", "armor_stand_marker", "armor_stand_small",
        "armor_stand_small_high", "armor_stand_nogravity", "armor_stand_invisible",
        "painting", "item_frame", "item_frame_empty", "item_frame_fixed",
        "glow_item_frame", "glow_item_frame_empty",
        "leash_knot", "leash_knot_tied", "falling_block", "cushion"
    };

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

    static JsonArray vecArr(Vec3 v) {
        JsonArray a = new JsonArray();
        a.add(v.x);
        a.add(v.y);
        a.add(v.z);
        return a;
    }

    static JsonArray vecBits(Vec3 v) {
        JsonArray a = new JsonArray();
        a.add(dBits(v.x));
        a.add(dBits(v.y));
        a.add(dBits(v.z));
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

        int activeProjId = -1;
        int activeTargetId = -1;
        int activePigId = -1;
        final List<JsonObject> tickSounds = new ArrayList<>();
        final List<JsonObject> tickEntityEvents = new ArrayList<>();
        final List<JsonObject> tickGameEvents = new ArrayList<>();

        Field fEntityRandom;
        Field fSharedFlags;
        Field fArrowFlags;
        Field fTridentDealt;
        Field fEntityEventId;
        Method mIsInGround;

        static Server create(Thread thread, LevelStorageSource.LevelStorageAccess access, PackRepository packs, Path output) throws Exception {
            packs.reload();
            WorldDataConfiguration config = WorldDataConfiguration.DEFAULT;
            LevelSettings settings = new LevelSettings("Witness", GameType.SPECTATOR, LevelSettings.DifficultySettings.DEFAULT, true, config);
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

        String roleOf(int id) {
            if (id == activeProjId) return "projectile";
            if (id == activeTargetId) return "target";
            if (id == activePigId) return "leashed_pig";
            if (player != null && id == player.getId()) return "player";
            return "other";
        }

        String roleOf(Entity e) {
            if (e == null) return null;
            String r = roleOf(e.getId());
            if (!r.equals("other")) return r;
            return BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString();
        }

        void runWitness() throws Exception {
            fEntityRandom = field(Entity.class, "random");
            fSharedFlags = field(Entity.class, "DATA_SHARED_FLAGS_ID");
            fArrowFlags = field(AbstractArrow.class, "ID_FLAGS");
            fTridentDealt = field(ThrownTrident.class, "dealtDamage");
            fEntityEventId = field(ClientboundEntityEventPacket.class, "entityId");
            mIsInGround = AbstractArrow.class.getDeclaredMethod("isInGround");
            mIsInGround.setAccessible(true);

            ServerLevel level = this.overworld();
            GameProfile profile = new GameProfile(UUIDUtil.createOfflinePlayerUUID("Player"), "Player");
            CommonListenerCookie cookie = CommonListenerCookie.createInitial(profile, false);
            player = new ServerPlayer(this, level, profile, ClientInformation.createDefault());
            Connection connection = new Connection(PacketFlow.SERVERBOUND);
            channel = new EmbeddedChannel(connection);
            channel.pipeline().addLast("witness_capture", new ChannelOutboundHandlerAdapter() {
                @Override
                public void write(ChannelHandlerContext ctx, Object msg, ChannelPromise promise) throws Exception {
                    if (msg instanceof ClientboundSoundPacket p) {
                        JsonObject s = new JsonObject();
                        s.addProperty("id", p.getSound().value().location().toString());
                        s.addProperty("source", p.getSource().getName());
                        s.addProperty("volume_bits", fBits(p.getVolume()));
                        s.addProperty("pitch_bits", fBits(p.getPitch()));
                        s.add("pos", vecArr(new Vec3(p.getX(), p.getY(), p.getZ())));
                        tickSounds.add(s);
                    } else if (msg instanceof ClientboundSoundEntityPacket p) {
                        JsonObject s = new JsonObject();
                        s.addProperty("id", p.getSound().value().location().toString());
                        s.addProperty("source", p.getSource().getName());
                        s.addProperty("entity_role", roleOf(p.getId()));
                        s.addProperty("volume_bits", fBits(p.getVolume()));
                        s.addProperty("pitch_bits", fBits(p.getPitch()));
                        tickSounds.add(s);
                    } else if (msg instanceof ClientboundExplodePacket p) {
                        JsonObject s = new JsonObject();
                        s.addProperty("id", p.explosionSound().value().location().toString());
                        s.addProperty("source", "explosion");
                        s.add("center_bits", vecBits(p.center()));
                        s.addProperty("radius_bits", fBits(p.radius()));
                        s.addProperty("play_sound", p.playSound());
                        tickSounds.add(s);
                    } else if (msg instanceof ClientboundEntityEventPacket p) {
                        int eid = fEntityEventId.getInt(p);
                        JsonObject ev = new JsonObject();
                        ev.addProperty("role", roleOf(eid));
                        ev.addProperty("event", (int) p.getEventId());
                        tickEntityEvents.add(ev);
                    }
                    super.write(ctx, msg, promise);
                }
            });
            this.getPlayerList().placeNewPlayer(connection, player, cookie);
            player.setGameMode(GameType.SPECTATOR);
            player.snapTo(0.5, 64.0, 12.5, 0.0f, 0.0f);
            settle();

            // Set deterministic gamerules.
            cmd("gamerule spawn_mobs false");
            cmd("gamerule advance_time false");
            cmd("gamerule advance_weather false");
            cmd("gamerule mob_griefing true");
            cmd("gamerule entity_drops true");
            cmd("gamerule random_tick_speed 0");

            // Register one GameEventListener on section (0, 4, 0) covering the arena.
            GameEventListener listener = new GameEventListener() {
                final PositionSource source = new BlockPositionSource(new BlockPos(3, 64, 4));
                @Override public PositionSource getListenerSource() { return source; }
                @Override public int getListenerRadius() { return 16; }
                @Override
                public boolean handleGameEvent(ServerLevel l, Holder<GameEvent> event, GameEvent.Context ctx, Vec3 pos) {
                    JsonObject ev = new JsonObject();
                    ev.addProperty("id", event.unwrapKey().map(k -> k.identifier().toString()).orElse("unknown"));
                    ev.add("pos_bits", vecBits(pos));
                    ev.addProperty("source", roleOf(ctx.sourceEntity()));
                    tickGameEvents.add(ev);
                    return false;
                }
            };
            level.getChunk(0, 0).getListenerRegistry(SectionPos.blockToSectionCoord(64)).register(listener);

            JsonObject root = new JsonObject();
            root.addProperty("game_version", SharedConstants.getCurrentVersion().id());
            root.addProperty("java_version", System.getProperty("java.version"));
            root.addProperty("ticks_per_case", TICKS);
            root.addProperty("projectile_seed", PROJECTILE_SEED);
            root.addProperty("level_seed", LEVEL_SEED);
            JsonArray cases = new JsonArray();

            for (String target : TARGETS) {
                for (String proj : PROJECTILES) {
                    cases.add(runCase(level, proj, target));
                }
            }
            // Owned projectile checks on key targets.
            String[] ownedTargets = {
                "armor_stand", "armor_stand_equipped", "painting",
                "item_frame", "item_frame_fixed", "glow_item_frame",
                "leash_knot_tied", "falling_block", "cushion"
            };
            for (String target : ownedTargets) {
                for (String proj : OWNED_PROJECTILES) {
                    cases.add(runCase(level, proj, target));
                }
            }
            // Mob-owned arrow hitting an item frame (BLOCK_CHANGE source attribution, finding #22).
            cases.add(runCase(level, "arrow_skeleton", "item_frame"));

            root.add("cases", cases);
            Files.createDirectories(output.getParent());
            Gson gson = new GsonBuilder().serializeNulls().create();
            Files.writeString(output, gson.toJson(root) + "\n", StandardCharsets.UTF_8);
            System.out.println("Wrote " + cases.size() + " cases to " + output);
        }

        void cmd(String text) {
            this.getCommands().performPrefixedCommand(this.createCommandSourceStack().withSuppressedOutput(), text);
        }

        void resetArena(ServerLevel level, String target) {
            // Discard all non-player entities.
            List<Entity> toDiscard = new ArrayList<>();
            for (Entity e : level.getAllEntities()) {
                if (e != player) toDiscard.add(e);
            }
            for (Entity e : toDiscard) e.discard();
            activeProjId = -1;
            activeTargetId = -1;
            activePigId = -1;

            // Rebuild floor, air volume, backstop wall, and target support blocks.
            for (int x = 0; x <= 9; x++) {
                for (int z = 0; z <= 9; z++) {
                    level.setBlock(new BlockPos(x, 63, z), Blocks.STONE.defaultBlockState(), 2);
                    for (int y = 64; y <= 70; y++) {
                        level.setBlock(new BlockPos(x, y, z), Blocks.AIR.defaultBlockState(), 2);
                    }
                }
            }
            for (int y = 64; y <= 67; y++) {
                for (int z = 2; z <= 6; z++) {
                    level.setBlock(new BlockPos(7, y, z), Blocks.STONE.defaultBlockState(), 2);
                }
            }
            boolean wallTarget = target.equals("painting") || target.startsWith("item_frame") || target.startsWith("glow_item_frame");
            if (wallTarget) {
                for (int y = 64; y <= 66; y++) {
                    for (int z = 2; z <= 6; z++) {
                        level.setBlock(new BlockPos(4, y, z), Blocks.STONE.defaultBlockState(), 2);
                    }
                }
            }
            if (target.startsWith("leash_knot")) {
                level.setBlock(new BlockPos(3, 64, 4), Blocks.OAK_FENCE.defaultBlockState(), 2);
            }
            player.setGameMode(GameType.SPECTATOR);
            player.snapTo(0.5, 64.0, 12.5, 0.0f, 0.0f);
            stepTick();
            channel.runPendingTasks();
            channel.outboundMessages().clear();
            tickSounds.clear();
            tickEntityEvents.clear();
            tickGameEvents.clear();
        }

        Entity findNewEntity(ServerLevel level, Set<Integer> before, Class<? extends Entity> cls) {
            for (Entity e : level.getAllEntities()) {
                if (!before.contains(e.getId()) && cls.isInstance(e)) return e;
            }
            return null;
        }

        Set<Integer> entityIds(ServerLevel level) {
            Set<Integer> ids = new HashSet<>();
            for (Entity e : level.getAllEntities()) ids.add(e.getId());
            return ids;
        }

        double spawnY(String target) {
            if (target.equals("armor_stand_small_high")) return 65.5;
            if (target.equals("falling_block")) return 65.3;
            if (target.startsWith("leash_knot")) return 64.6;
            if (target.equals("cushion")) return 64.15;
            return 64.5;
        }

        JsonObject runCase(ServerLevel level, String projKind, String targetKind) throws Exception {
            resetArena(level, targetKind);
            level.getRandom().setSeed(LEVEL_SEED);

            // Summon target.
            Set<Integer> beforeTarget = entityIds(level);
            Pig leashedPig = null;
            switch (targetKind) {
                case "armor_stand" -> cmd("summon minecraft:armor_stand 3.5 64.0 4.5 {}");
                case "armor_stand_equipped" -> cmd("summon minecraft:armor_stand 3.5 64.0 4.5 {equipment:{head:{id:\"minecraft:iron_helmet\",count:1}}}");
                case "armor_stand_marker" -> cmd("summon minecraft:armor_stand 3.5 64.0 4.5 {Marker:1b}");
                case "armor_stand_small" -> cmd("summon minecraft:armor_stand 3.5 64.0 4.5 {Small:1b}");
                case "armor_stand_small_high" -> cmd("summon minecraft:armor_stand 3.5 64.0 4.5 {Small:1b}");
                case "armor_stand_nogravity" -> cmd("summon minecraft:armor_stand 3.5 64.0 4.5 {NoGravity:1b}");
                case "armor_stand_invisible" -> cmd("summon minecraft:armor_stand 3.5 64.0 4.5 {Invisible:1b}");
                case "painting" -> cmd("summon minecraft:painting 3.5 64.5 4.5 {block_pos:[I;3,64,4],facing:1b,variant:\"minecraft:kebab\"}");
                case "item_frame" -> cmd("summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:\"minecraft:diamond\",count:1}}");
                case "item_frame_empty" -> cmd("summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b}");
                case "item_frame_fixed" -> cmd("summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Fixed:1b,Item:{id:\"minecraft:diamond\",count:1}}");
                case "glow_item_frame" -> cmd("summon minecraft:glow_item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:\"minecraft:diamond\",count:1}}");
                case "glow_item_frame_empty" -> cmd("summon minecraft:glow_item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b}");
                case "leash_knot" -> cmd("summon minecraft:leash_knot 3.5 64.375 4.5 {}");
                case "leash_knot_tied" -> {
                    cmd("summon minecraft:pig 3.5 64.0 6.5 {NoAI:1b,Silent:1b,leash:[I;3,64,4]}");
                    stepTick();
                    channel.runPendingTasks();
                    channel.outboundMessages().clear();
                    tickSounds.clear();
                    tickEntityEvents.clear();
                    tickGameEvents.clear();
                }
                case "falling_block" -> cmd("summon minecraft:falling_block 3.5 65.0 4.5 {BlockState:\"minecraft:sand\",Time:1,Motion:[0.0d,0.0d,0.0d]}");
                case "cushion" -> cmd("summon minecraft:cushion 3.5 64.0 4.5 {block_pos:[I;3,64,4],color:\"white\"}");
                default -> throw new IllegalArgumentException(targetKind);
            }
            Entity target;
            if (targetKind.equals("leash_knot_tied")) {
                leashedPig = (Pig) findNewEntity(level, beforeTarget, Pig.class);
                if (leashedPig == null) throw new IllegalStateException("pig failed to spawn");
                activePigId = leashedPig.getId();
                target = findNewEntity(level, beforeTarget, LeashFenceKnotEntity.class);
            } else {
                target = findNewEntity(level, beforeTarget, Entity.class);
            }
            if (target == null) throw new IllegalStateException("target failed to spawn: " + targetKind);
            activeTargetId = target.getId();
            fEntityRandom.set(target, RandomSource.create(PROJECTILE_SEED));

            // Configure owner gamemode or mob owner if this is an owned projectile case.
            boolean owned = projKind.endsWith("_owned") || projKind.endsWith("_creative");
            boolean mobOwned = projKind.equals("arrow_skeleton");
            Entity mobOwner = null;
            if (projKind.endsWith("_creative")) {
                player.setGameMode(GameType.CREATIVE);
            } else if (projKind.endsWith("_owned")) {
                player.setGameMode(GameType.SURVIVAL);
            } else if (mobOwned) {
                Set<Integer> beforeMob = entityIds(level);
                cmd("summon minecraft:skeleton 0.5 64.0 2.5 {NoAI:1b,Silent:1b}");
                mobOwner = findNewEntity(level, beforeMob, Entity.class);
                if (mobOwner == null) throw new IllegalStateException("skeleton failed to spawn");
            }

            // Summon projectile.
            double y0 = spawnY(targetKind);
            Set<Integer> beforeProj = entityIds(level);
            String baseKind = projKind.replace("_creative", "").replace("_owned", "").replace("_skeleton", "");
            switch (baseKind) {
                case "arrow" -> cmd("summon minecraft:arrow 1.5 " + y0 + " 4.5 {pickup:1b,Motion:[1.0d,0.0d,0.0d]}");
                case "spectral_arrow" -> cmd("summon minecraft:spectral_arrow 1.5 " + y0 + " 4.5 {pickup:1b,Motion:[1.0d,0.0d,0.0d]}");
                case "trident" -> cmd("summon minecraft:trident 1.5 " + y0 + " 4.5 {pickup:1b,Motion:[1.0d,0.0d,0.0d],item:{id:\"minecraft:trident\",count:1}}");
                case "snowball" -> cmd("summon minecraft:snowball 1.5 " + y0 + " 4.5 {Motion:[1.0d,0.0d,0.0d]}");
                case "egg" -> cmd("summon minecraft:egg 1.5 " + y0 + " 4.5 {Motion:[1.0d,0.0d,0.0d]}");
                case "ender_pearl" -> cmd("summon minecraft:ender_pearl 1.5 " + y0 + " 4.5 {Motion:[1.0d,0.0d,0.0d]}");
                case "wind_charge" -> cmd("summon minecraft:wind_charge 1.5 " + y0 + " 4.5 {acceleration_power:0.0d,Motion:[1.0d,0.0d,0.0d]}");
                case "small_fireball" -> cmd("summon minecraft:small_fireball 1.5 " + y0 + " 4.5 {acceleration_power:0.1d,Motion:[1.0d,0.0d,0.0d]}");
                case "fireball" -> cmd("summon minecraft:fireball 1.5 " + y0 + " 4.5 {ExplosionPower:1b,acceleration_power:0.1d,Motion:[1.0d,0.0d,0.0d]}");
                default -> throw new IllegalArgumentException(projKind);
            }
            Projectile proj = (Projectile) findNewEntity(level, beforeProj, Projectile.class);
            if (proj == null) throw new IllegalStateException("projectile failed to spawn: " + projKind);
            activeProjId = proj.getId();
            if (owned) proj.setOwner(player);
            if (mobOwner != null) proj.setOwner(mobOwner);
            fEntityRandom.set(proj, RandomSource.create(PROJECTILE_SEED));
            level.getRandom().setSeed(LEVEL_SEED);

            // Flush spawn tracking packets before tick 1.
            channel.runPendingTasks();
            channel.outboundMessages().clear();
            tickSounds.clear();
            tickEntityEvents.clear();
            tickGameEvents.clear();

            long startTime = level.getGameTime();
            Set<Integer> knownEntities = entityIds(level);

            JsonObject rec = new JsonObject();
            rec.addProperty("projectile", projKind);
            rec.addProperty("target", targetKind);
            rec.addProperty("spawn_y", y0);
            rec.addProperty("target_can_be_hit", target.canBeHitByProjectile());
            rec.addProperty("target_is_pickable", target.isPickable());
            rec.addProperty("target_no_physics", target.noPhysics);
            JsonArray targetBox = new JsonArray();
            var bb = target.getBoundingBox();
            targetBox.add(bb.minX); targetBox.add(bb.minY); targetBox.add(bb.minZ);
            targetBox.add(bb.maxX); targetBox.add(bb.maxY); targetBox.add(bb.maxZ);
            rec.add("target_initial_box", targetBox);

            JsonArray ticksArr = new JsonArray();
            ticksArr.add(snapshotTick(0, level, proj, target, leashedPig, startTime, List.of(), List.of()));

            Integer hitTick = null;
            for (int t = 1; t <= TICKS; t++) {
                tickSounds.clear();
                tickEntityEvents.clear();
                tickGameEvents.clear();
                boolean projWasAlive = !proj.isRemoved();
                boolean targetWasAlive = !target.isRemoved();
                Vec3 projPrevVel = proj.getDeltaMovement();

                stepTick();
                channel.runPendingTasks();
                channel.outboundMessages().clear();

                List<JsonObject> newDrops = new ArrayList<>();
                List<String> newSpawned = new ArrayList<>();
                for (Entity e : level.getAllEntities()) {
                    if (knownEntities.add(e.getId())) {
                        if (e instanceof ItemEntity item) {
                            JsonObject d = new JsonObject();
                            d.addProperty("item", BuiltInRegistries.ITEM.getKey(item.getItem().getItem()).toString());
                            d.addProperty("count", item.getItem().getCount());
                            newDrops.add(d);
                            // Discard recorded drops so unseeded item scatter never drifts into fire later.
                            item.discard();
                        } else {
                            newSpawned.add(BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString());
                        }
                    }
                }

                // Detect first tick where projectile impacted something (stopped, removed, bounced, or hit target).
                if (hitTick == null && projWasAlive) {
                    boolean projGone = proj.isRemoved();
                    boolean inGround = proj instanceof AbstractArrow a && (boolean) mIsInGround.invoke(a);
                    boolean bounced = proj.getDeltaMovement().x < 0.0 && projPrevVel.x > 0.0;
                    boolean targetDied = targetWasAlive && target.isRemoved();
                    if (projGone || inGround || bounced || targetDied) {
                        hitTick = t;
                    }
                }
                ticksArr.add(snapshotTick(t, level, proj, target, leashedPig, startTime, newDrops, newSpawned));
            }
            rec.add("hit_tick", hitTick == null ? JsonNull.INSTANCE : new com.google.gson.JsonPrimitive(hitTick));
            rec.add("ticks", ticksArr);
            return rec;
        }

        @SuppressWarnings("unchecked")
        JsonObject snapshotTick(int t, ServerLevel level, Projectile proj, Entity target, Pig pig,
                                long startTime, List<JsonObject> drops, List<String> spawned) throws Exception {
            JsonObject row = new JsonObject();
            row.addProperty("tick", t);

            JsonObject pObj = new JsonObject();
            pObj.addProperty("alive", !proj.isRemoved());
            pObj.add("pos", vecArr(proj.position()));
            pObj.add("pos_bits", vecBits(proj.position()));
            pObj.add("vel", vecArr(proj.getDeltaMovement()));
            pObj.add("vel_bits", vecBits(proj.getDeltaMovement()));
            pObj.addProperty("yaw_bits", fBits(proj.getYRot()));
            pObj.addProperty("pitch_bits", fBits(proj.getXRot()));
            pObj.addProperty("fire_ticks", proj.getRemainingFireTicks());
            pObj.addProperty("on_ground", proj.onGround());
            EntityDataAccessor<Byte> sharedAcc = (EntityDataAccessor<Byte>) fSharedFlags.get(null);
            pObj.addProperty("shared_flags", (int) proj.getEntityData().get(sharedAcc));
            if (proj instanceof AbstractArrow arrow) {
                EntityDataAccessor<Byte> arrowAcc = (EntityDataAccessor<Byte>) fArrowFlags.get(null);
                pObj.addProperty("arrow_flags", (int) arrow.getEntityData().get(arrowAcc));
                pObj.addProperty("in_ground", (boolean) mIsInGround.invoke(arrow));
                pObj.addProperty("shake", arrow.shakeTime);
                pObj.addProperty("crit", arrow.isCritArrow());
                pObj.addProperty("no_physics", arrow.isNoPhysics());
                if (arrow instanceof ThrownTrident trident) {
                    pObj.addProperty("dealt_damage", fTridentDealt.getBoolean(trident));
                }
            }
            row.add("projectile", pObj);

            JsonObject tObj = new JsonObject();
            tObj.addProperty("alive", !target.isRemoved());
            tObj.add("pos", vecArr(target.position()));
            tObj.add("pos_bits", vecBits(target.position()));
            tObj.add("vel", vecArr(target.getDeltaMovement()));
            tObj.add("vel_bits", vecBits(target.getDeltaMovement()));
            tObj.addProperty("fire_ticks", target.getRemainingFireTicks());
            if (target instanceof ArmorStand stand) {
                tObj.addProperty("health", stand.getHealth());
                tObj.add("last_hit_delta", stand.lastHit == 0L ? JsonNull.INSTANCE
                    : new com.google.gson.JsonPrimitive(stand.lastHit - startTime));
                JsonObject eq = new JsonObject();
                for (EquipmentSlot slot : EquipmentSlot.values()) {
                    ItemStack st = stand.getItemBySlot(slot);
                    if (!st.isEmpty()) eq.addProperty(slot.getName(), BuiltInRegistries.ITEM.getKey(st.getItem()).toString());
                }
                tObj.add("equipment", eq);
            } else if (target instanceof ItemFrame frame) {
                ItemStack st = frame.getItem();
                tObj.add("item", st.isEmpty() ? JsonNull.INSTANCE
                    : new com.google.gson.JsonPrimitive(BuiltInRegistries.ITEM.getKey(st.getItem()).toString()));
            } else if (target instanceof FallingBlockEntity fb) {
                tObj.addProperty("time", fb.time);
                tObj.addProperty("on_ground", fb.onGround());
                tObj.addProperty("block_at_floor", BuiltInRegistries.BLOCK.getKey(
                    level.getBlockState(new BlockPos(3, 64, 4)).getBlock()).toString());
            } else if (target instanceof LeashFenceKnotEntity && pig != null) {
                tObj.addProperty("pig_leashed", pig.isLeashed());
            }
            row.add("target", tObj);

            JsonArray sounds = new JsonArray();
            tickSounds.forEach(sounds::add);
            row.add("sounds", sounds);

            JsonArray eEvents = new JsonArray();
            tickEntityEvents.forEach(eEvents::add);
            row.add("entity_events", eEvents);

            JsonArray gEvents = new JsonArray();
            tickGameEvents.forEach(gEvents::add);
            row.add("game_events", gEvents);

            JsonArray dropsArr = new JsonArray();
            drops.forEach(dropsArr::add);
            row.add("drops", dropsArr);

            JsonArray spawnedArr = new JsonArray();
            spawned.forEach(spawnedArr::add);
            row.add("spawned", spawnedArr);

            return row;
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
            for (int dz = -2; dz <= 2; dz++) {
                for (int dx = -2; dx <= 2; dx++) {
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
                for (int dz = -2; dz <= 2 && ready; dz++) {
                    for (int dx = -2; dx <= 2 && ready; dx++) {
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
            universe = Files.createTempDirectory("sunrise-projectile-targets-oracle-");
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
