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
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.UUID;
import java.util.function.BooleanSupplier;
import net.minecraft.SharedConstants;
import net.minecraft.SystemReport;
import net.minecraft.advancements.AdvancementHolder;
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
import net.minecraft.network.protocol.game.ClientboundSoundEntityPacket;
import net.minecraft.network.protocol.game.ClientboundSoundPacket;
import net.minecraft.resources.Identifier;
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
import net.minecraft.world.entity.LightningBolt;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.animal.pig.Pig;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.minecraft.world.entity.decoration.ItemFrame;
import net.minecraft.world.entity.decoration.LeashFenceKnotEntity;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.projectile.FireworkRocketEntity;
import net.minecraft.world.entity.projectile.Projectile;
import net.minecraft.world.entity.projectile.arrow.AbstractArrow;
import net.minecraft.world.entity.projectile.arrow.ThrownTrident;
import net.minecraft.world.entity.vehicle.boat.AbstractBoat;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.SculkSensorBlock;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
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

// Witnesses projectile parity gaps #120-#124 in a real vanilla 26.3 ServerLevel.
public final class VanillaProjectilesParityOracle {
    static final long PROJ_SEED = 12345L;
    static final long LEVEL_SEED = 67890L;

    static final String STAR_FIREWORK_ITEM =
        "{id:\"minecraft:firework_rocket\",count:1,components:{\"minecraft:fireworks\":{flight_duration:1,explosions:[{shape:\"small_ball\",colors:[I;11743532]}]}}}";
    static final String PLAIN_FIREWORK_ITEM =
        "{id:\"minecraft:firework_rocket\",count:1}";

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
        final List<JsonObject> tickGameEvents = new ArrayList<>();
        final List<String> tickSounds = new ArrayList<>();

        Field fEntityRandom;
        Field fDealtDamage;

        static Server create(Thread thread, LevelStorageSource.LevelStorageAccess access, PackRepository packs, Path output) throws Exception {
            packs.reload();
            WorldDataConfiguration config = WorldDataConfiguration.DEFAULT;
            LevelSettings settings = new LevelSettings("Proj120_124", GameType.SURVIVAL, LevelSettings.DifficultySettings.DEFAULT, true, config);
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

        String uuidNbt(Entity e) {
            int[] arr = UUIDUtil.uuidToIntArray(e.getUUID());
            return "[I;" + arr[0] + "," + arr[1] + "," + arr[2] + "," + arr[3] + "]";
        }

        String playerUuidNbt() {
            return uuidNbt(player);
        }

        boolean advDone(String id) {
            AdvancementHolder h = this.getAdvancements().get(Identifier.parse(id));
            return h != null && player.getAdvancements().getOrStartProgress(h).isDone();
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

        void clearArena(ServerLevel level, int backstopX) {
            if (player.isPassenger()) {
                player.stopRiding();
            }
            List<Entity> toDiscard = new ArrayList<>();
            for (Entity e : level.getAllEntities()) {
                if (e != player) toDiscard.add(e);
            }
            for (Entity e : toDiscard) e.discard();
            int maxX = Math.max(14, backstopX + 2);
            for (int x = 0; x <= maxX; x++) {
                for (int z = 0; z <= 10; z++) {
                    level.setBlock(new BlockPos(x, 63, z), Blocks.STONE.defaultBlockState(), 2);
                    for (int y = 64; y <= 70; y++) {
                        level.setBlock(new BlockPos(x, y, z), Blocks.AIR.defaultBlockState(), 2);
                    }
                }
            }
            for (int x = 3; x <= 5; x++) {
                for (int z = 16; z <= 18; z++) {
                    level.setBlock(new BlockPos(x, 63, z), Blocks.STONE.defaultBlockState(), 2);
                    for (int y = 64; y <= 67; y++) {
                        level.setBlock(new BlockPos(x, y, z), Blocks.AIR.defaultBlockState(), 2);
                    }
                }
            }
            if (backstopX > 0) {
                for (int y = 64; y <= 67; y++) {
                    for (int z = 2; z <= 6; z++) {
                        level.setBlock(new BlockPos(backstopX, y, z), Blocks.STONE.defaultBlockState(), 2);
                    }
                }
            }
            cmd("gamerule pvp true");
            cmd("team remove blue");
            cmd("weather clear 10000");
            level.setRainLevel(0.0f);
            level.setThunderLevel(0.0f);
            player.setGameMode(GameType.SURVIVAL);
            player.setHealth(player.getMaxHealth());
            clearInvuln();
            player.snapTo(4.5, 64.0, 17.5, -90.0f, 0.0f);
            player.setDeltaMovement(Vec3.ZERO);
            cmd("advancement revoke @a everything");
            stepTick();
            drainChannel();
            tickGameEvents.clear();
            tickSounds.clear();
        }

        void drainChannel() {
            channel.runPendingTasks();
            channel.outboundMessages().clear();
        }

        void seedProj(Entity e) throws Exception {
            fEntityRandom.set(e, RandomSource.create(PROJ_SEED));
        }

        void clearInvuln() {
            player.setInvulnerableTime(0);
        }

        JsonArray snapEvents() {
            JsonArray a = new JsonArray();
            for (JsonObject ev : tickGameEvents) a.add(ev);
            return a;
        }

        JsonArray snapSounds() {
            JsonArray a = new JsonArray();
            for (String s : tickSounds) a.add(s);
            return a;
        }

        JsonArray snapDrops(ServerLevel level) {
            List<String> items = new ArrayList<>();
            for (Entity e : level.getAllEntities()) {
                if (e instanceof ItemEntity ie && ie.isAlive()) {
                    items.add(BuiltInRegistries.ITEM.getKey(ie.getItem().getItem()).toString());
                }
            }
            items.sort(String::compareTo);
            JsonArray a = new JsonArray();
            for (String s : items) a.add(s);
            return a;
        }

        void runWitness() throws Exception {
            fEntityRandom = field(Entity.class, "random");
            fDealtDamage = field(ThrownTrident.class, "dealtDamage");

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
            player.setGameMode(GameType.SURVIVAL);
            player.snapTo(0.5, 64.0, 9.5, -90.0f, 0.0f);
            settle();

            cmd("gamerule spawn_mobs false");
            cmd("gamerule advance_time false");
            cmd("gamerule advance_weather false");
            cmd("gamerule mob_griefing true");
            cmd("gamerule mob_drops false");
            cmd("gamerule entity_drops true");
            cmd("gamerule random_tick_speed 0");

            GameEventListener listener = new GameEventListener() {
                final PositionSource source = new BlockPositionSource(new BlockPos(5, 64, 4));
                @Override public PositionSource getListenerSource() { return source; }
                @Override public int getListenerRadius() { return 16; }
                @Override
                public boolean handleGameEvent(ServerLevel l, Holder<GameEvent> event, GameEvent.Context ctx, Vec3 pos) {
                    JsonObject ev = new JsonObject();
                    ev.addProperty("id", event.unwrapKey().map(k -> k.identifier().toString()).orElse("unknown"));
                    ev.add("pos_bits", vecBits(pos));
                    Entity src = ctx.sourceEntity();
                    ev.addProperty("source", src == null ? null : (src == player ? "player" : BuiltInRegistries.ENTITY_TYPE.getKey(src.getType()).toString()));
                    tickGameEvents.add(ev);
                    return false;
                }
            };
            level.getChunk(0, 0).getListenerRegistry(SectionPos.blockToSectionCoord(64)).register(listener);

            JsonObject root = new JsonObject();
            root.addProperty("game_version", SharedConstants.getCurrentVersion().id());
            root.addProperty("java_version", System.getProperty("java.version"));
            root.addProperty("proj_seed", PROJ_SEED);
            root.addProperty("level_seed", LEVEL_SEED);
            root.add("gap_120_spectator", run120(level));
            root.add("gap_121_wind_crystal", run121(level));
            root.add("gap_122_trident_enderman", run122(level));
            root.add("gap_123_piercing", run123(level));
            root.add("gap_124_fireworks", run124(level));
            root.add("review_round1", runReviewRound1(level));

            Files.createDirectories(output.getParent());
            Gson gson = new GsonBuilder().serializeNulls().create();
            Files.writeString(output, gson.toJson(root) + "\n", StandardCharsets.UTF_8);
            System.out.println("Wrote #120-#124 witness to " + output);
        }

        // #120: spectator vs survival host player as projectile target.
        JsonArray run120(ServerLevel level) throws Exception {
            JsonArray out = new JsonArray();
            String[] modes = {"spectator", "survival"};
            String[] projs = {"arrow", "snowball", "small_fireball", "wind_charge", "trident"};
            for (String mode : modes) {
                for (String proj : projs) {
                    clearArena(level, 8);
                    level.getRandom().setSeed(LEVEL_SEED);
                    player.setGameMode(mode.equals("spectator") ? GameType.SPECTATOR : GameType.SURVIVAL);
                    player.setHealth(player.getMaxHealth());
                    clearInvuln();
                    player.snapTo(3.5, 64.0, 4.5, -90.0f, 0.0f);
                    player.setDeltaMovement(Vec3.ZERO);
                    stepTick();
                    drainChannel();

                    Set<Integer> before = entityIds(level);
                    switch (proj) {
                        case "arrow" -> cmd("summon minecraft:arrow 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],damage:2.0d}");
                        case "snowball" -> cmd("summon minecraft:snowball 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d]}");
                        case "small_fireball" -> cmd("summon minecraft:small_fireball 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],acceleration_power:0.0d}");
                        case "wind_charge" -> cmd("summon minecraft:wind_charge 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],acceleration_power:0.0d}");
                        case "trident" -> cmd("summon minecraft:trident 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d]}");
                        default -> throw new IllegalArgumentException(proj);
                    }
                    Projectile p = findNew(level, before, Projectile.class);
                    seedProj(p);
                    tickGameEvents.clear();
                    tickSounds.clear();

                    JsonObject rec = new JsonObject();
                    rec.addProperty("mode", mode);
                    rec.addProperty("projectile", proj);
                    JsonArray ticks = new JsonArray();
                    for (int t = 1; t <= 8; t++) {
                        tickGameEvents.clear();
                        tickSounds.clear();
                        stepTick();
                        drainChannel();
                        JsonObject row = new JsonObject();
                        row.addProperty("tick", t);
                        row.addProperty("proj_alive", !p.isRemoved());
                        row.add("proj_pos_bits", vecBits(p.position()));
                        row.add("proj_vel_bits", vecBits(p.getDeltaMovement()));
                        row.addProperty("in_ground", p instanceof AbstractArrow a && a.isNoPhysics() ? false : (p instanceof AbstractArrow a2 && a2.getDeltaMovement().lengthSqr() == 0.0 && !p.isRemoved()));
                        row.addProperty("player_health", player.getHealth());
                        row.add("game_events", snapEvents());
                        row.add("sounds", snapSounds());
                        ticks.add(row);
                    }
                    rec.add("ticks", ticks);
                    out.add(rec);
                }
            }
            return out;
        }

        // #121: wind charge & breeze wind charge passing through End crystal vs arrow hitting it.
        JsonArray run121(ServerLevel level) throws Exception {
            JsonArray out = new JsonArray();
            String[] cases = {
                "wind_charge_through_crystal",
                "breeze_wind_charge_through_crystal",
                "wind_charge_crystal_then_pig",
                "arrow_hits_crystal"
            };
            for (String name : cases) {
                clearArena(level, 9);
                level.getRandom().setSeed(LEVEL_SEED);
                Set<Integer> before = entityIds(level);
                cmd("summon minecraft:end_crystal 4.25 64.0 4.5 {}");
                Entity crystal = findNew(level, before, Entity.class);

                LivingEntity pig = null;
                if (name.equals("wind_charge_crystal_then_pig")) {
                    Set<Integer> bp = entityIds(level);
                    cmd("summon minecraft:pig 7.5 64.0 4.5 {NoAI:1b,Silent:1b}");
                    pig = findNew(level, bp, LivingEntity.class);
                }

                Set<Integer> bp2 = entityIds(level);
                switch (name) {
                    case "wind_charge_through_crystal", "wind_charge_crystal_then_pig" ->
                        cmd("summon minecraft:wind_charge 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],acceleration_power:0.0d}");
                    case "breeze_wind_charge_through_crystal" ->
                        cmd("summon minecraft:breeze_wind_charge 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],acceleration_power:0.0d}");
                    case "arrow_hits_crystal" ->
                        cmd("summon minecraft:arrow 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],damage:2.0d}");
                    default -> throw new IllegalArgumentException(name);
                }
                Projectile p = findNew(level, bp2, Projectile.class);
                seedProj(p);

                JsonObject rec = new JsonObject();
                rec.addProperty("case", name);
                JsonArray ticks = new JsonArray();
                for (int t = 1; t <= 8; t++) {
                    tickGameEvents.clear();
                    tickSounds.clear();
                    stepTick();
                    drainChannel();
                    JsonObject row = new JsonObject();
                    row.addProperty("tick", t);
                    row.addProperty("proj_alive", !p.isRemoved());
                    row.add("proj_pos_bits", vecBits(p.position()));
                    row.add("proj_vel_bits", vecBits(p.getDeltaMovement()));
                    row.addProperty("crystal_alive", !crystal.isRemoved());
                    row.addProperty("pig_health", pig == null ? -1.0f : pig.getHealth());
                    row.add("game_events", snapEvents());
                    row.add("sounds", snapSounds());
                    ticks.add(row);
                }
                rec.add("ticks", ticks);
                out.add(rec);
            }
            return out;
        }

        // #122: trident vs enderman (dodge without sound/bounce, dealtDamage=true) vs boat/invulnerable controls.
        JsonArray run122(ServerLevel level) throws Exception {
            JsonArray out = new JsonArray();
            String[] cases = {
                "trident_unowned_enderman",
                "trident_owned_enderman",
                "trident_enderman_in_boat",
                "trident_invulnerable_pig"
            };
            for (String name : cases) {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);

                Set<Integer> bt = entityIds(level);
                LivingEntity target;
                if (name.equals("trident_enderman_in_boat")) {
                    cmd("summon minecraft:oak_boat 3.5 64.0 4.5 {Passengers:[{id:\"minecraft:enderman\",NoAI:1b,Silent:1b}]}");
                    target = findNew(level, bt, LivingEntity.class);
                } else if (name.equals("trident_invulnerable_pig")) {
                    cmd("summon minecraft:pig 3.5 64.0 4.5 {NoAI:1b,Silent:1b,Invulnerable:1b}");
                    target = findNew(level, bt, LivingEntity.class);
                } else {
                    cmd("summon minecraft:enderman 3.5 64.0 4.5 {NoAI:1b,Silent:1b}");
                    target = findNew(level, bt, LivingEntity.class);
                }
                Vec3 targetPos0 = target.position();

                Set<Integer> bpig = entityIds(level);
                cmd("summon minecraft:pig 6.5 64.0 4.5 {NoAI:1b,Silent:1b}");
                LivingEntity behind = findNew(level, bpig, LivingEntity.class);

                Set<Integer> bp = entityIds(level);
                if (name.equals("trident_owned_enderman")) {
                    cmd("summon minecraft:trident 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],Owner:" + playerUuidNbt() + ",LeftOwner:1b}");
                } else {
                    cmd("summon minecraft:trident 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d]}");
                }
                ThrownTrident tr = findNew(level, bp, ThrownTrident.class);
                seedProj(tr);

                JsonObject rec = new JsonObject();
                rec.addProperty("case", name);
                JsonArray ticks = new JsonArray();
                for (int t = 1; t <= 8; t++) {
                    tickGameEvents.clear();
                    tickSounds.clear();
                    stepTick();
                    drainChannel();
                    JsonObject row = new JsonObject();
                    row.addProperty("tick", t);
                    row.addProperty("proj_alive", !tr.isRemoved());
                    row.add("proj_pos_bits", vecBits(tr.position()));
                    row.add("proj_vel_bits", vecBits(tr.getDeltaMovement()));
                    row.addProperty("proj_yaw_bits", fBits(tr.getYRot()));
                    row.addProperty("dealt_damage", fDealtDamage.getBoolean(tr));
                    row.addProperty("target_health", target.getHealth());
                    row.addProperty("target_teleported", target.position().distanceToSqr(targetPos0) > 1e-6);
                    row.addProperty("behind_pig_health", behind.getHealth());
                    row.add("game_events", snapEvents());
                    row.add("sounds", snapSounds());
                    ticks.add(row);
                }
                rec.add("ticks", ticks);
                out.add(rec);
            }
            return out;
        }

        // #123: piercing arrow multi-hit in one tick + killed_by_arrow aggregation.
        JsonArray run123(ServerLevel level) throws Exception {
            JsonArray out = new JsonArray();
            String[] cases = {
                "pierce_two_mobs_same_tick",
                "pierce_two_phantoms_two_birds",
                "pierce_five_unique_arbalistic",
                "pierce_limit_stops_at_pierce_plus_one",
                "pierce_through_mob_into_wall_same_tick",
                "pierce_hits_falling_block_deflects",
                "pierce_hits_breeze_deflects"
            };
            String owner = playerUuidNbt();
            String xbow = "{id:\"minecraft:crossbow\",count:1}";
            for (String name : cases) {
                int wallX = name.equals("pierce_through_mob_into_wall_same_tick") ? 4 : 11;
                clearArena(level, wallX);
                level.getRandom().setSeed(LEVEL_SEED);

                List<Entity> tracked = new ArrayList<>();
                switch (name) {
                    case "pierce_two_mobs_same_tick" -> {
                        tracked.add(summon(level, "summon minecraft:chicken 2.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                        tracked.add(summon(level, "summon minecraft:pig 3.8 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                    }
                    case "pierce_two_phantoms_two_birds" -> {
                        tracked.add(summon(level, "summon minecraft:phantom 2.5 64.3 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                        tracked.add(summon(level, "summon minecraft:phantom 3.8 64.3 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                    }
                    case "pierce_five_unique_arbalistic" -> {
                        tracked.add(summon(level, "summon minecraft:pig 2.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                        tracked.add(summon(level, "summon minecraft:cow 3.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                        tracked.add(summon(level, "summon minecraft:sheep 4.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                        tracked.add(summon(level, "summon minecraft:chicken 5.8 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                        tracked.add(summon(level, "summon minecraft:villager 7.0 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                    }
                    case "pierce_limit_stops_at_pierce_plus_one" -> {
                        tracked.add(summon(level, "summon minecraft:pig 2.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                        tracked.add(summon(level, "summon minecraft:pig 3.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                        tracked.add(summon(level, "summon minecraft:pig 4.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                    }
                    case "pierce_through_mob_into_wall_same_tick" -> {
                        tracked.add(summon(level, "summon minecraft:pig 2.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                    }
                    case "pierce_hits_falling_block_deflects" -> {
                        tracked.add(summon(level, "summon minecraft:falling_block 2.5 65.0 4.5 {BlockState:\"minecraft:sand\",Time:1,Motion:[0.0d,0.0d,0.0d]}"));
                        tracked.add(summon(level, "summon minecraft:zombie 3.8 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                    }
                    case "pierce_hits_breeze_deflects" -> {
                        tracked.add(summon(level, "summon minecraft:breeze 2.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
                        tracked.add(summon(level, "summon minecraft:pig 3.8 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"));
                    }
                    default -> throw new IllegalArgumentException(name);
                }

                Set<Integer> bp = entityIds(level);
                switch (name) {
                    case "pierce_two_mobs_same_tick", "pierce_two_phantoms_two_birds", "pierce_through_mob_into_wall_same_tick", "pierce_hits_breeze_deflects" ->
                        cmd("summon minecraft:arrow 1.5 64.5 4.5 {Motion:[3.0d,0.0d,0.0d],damage:20.0d,PierceLevel:2b,Owner:" + owner + ",LeftOwner:1b,weapon:" + xbow + "}");
                    case "pierce_five_unique_arbalistic" ->
                        cmd("summon minecraft:arrow 1.5 64.5 4.5 {Motion:[3.5d,0.0d,0.0d],damage:20.0d,PierceLevel:4b,Owner:" + owner + ",LeftOwner:1b,weapon:" + xbow + "}");
                    case "pierce_limit_stops_at_pierce_plus_one" ->
                        cmd("summon minecraft:arrow 1.5 64.5 4.5 {Motion:[4.0d,0.0d,0.0d],damage:20.0d,PierceLevel:1b,Owner:" + owner + ",LeftOwner:1b,weapon:" + xbow + "}");
                    case "pierce_hits_falling_block_deflects" ->
                        cmd("summon minecraft:arrow 1.5 65.3 4.5 {Motion:[3.0d,0.0d,0.0d],damage:20.0d,PierceLevel:2b,Owner:" + owner + ",LeftOwner:1b,weapon:" + xbow + "}");
                    default -> throw new IllegalArgumentException(name);
                }
                AbstractArrow arrow = findNew(level, bp, AbstractArrow.class);
                seedProj(arrow);

                JsonObject rec = new JsonObject();
                rec.addProperty("case", name);
                JsonArray ticks = new JsonArray();
                for (int t = 1; t <= 4; t++) {
                    tickGameEvents.clear();
                    tickSounds.clear();
                    stepTick();
                    drainChannel();
                    JsonObject row = new JsonObject();
                    row.addProperty("tick", t);
                    row.addProperty("proj_alive", !arrow.isRemoved());
                    row.add("proj_pos_bits", vecBits(arrow.position()));
                    row.add("proj_vel_bits", vecBits(arrow.getDeltaMovement()));
                    row.addProperty("proj_yaw_bits", fBits(arrow.getYRot()));
                    row.addProperty("in_ground", arrow.getDeltaMovement().lengthSqr() == 0.0 && !arrow.isRemoved());
                    JsonArray mobs = new JsonArray();
                    for (Entity e : tracked) {
                        JsonObject m = new JsonObject();
                        m.addProperty("kind", BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString());
                        m.addProperty("alive", e.isAlive());
                        m.addProperty("health", e instanceof LivingEntity le ? le.getHealth() : -1.0f);
                        mobs.add(m);
                    }
                    row.add("targets", mobs);
                    row.addProperty("two_birds_done", advDone("minecraft:adventure/two_birds_one_arrow"));
                    row.addProperty("arbalistic_done", advDone("minecraft:adventure/arbalistic"));
                    row.add("game_events", snapEvents());
                    row.add("sounds", snapSounds());
                    ticks.add(row);
                }
                rec.add("ticks", ticks);
                out.add(rec);
            }
            return out;
        }

        // #124: firework rocket contact with pickable non-mobs + blast damaging armor stands.
        JsonArray run124(ServerLevel level) throws Exception {
            JsonArray out = new JsonArray();
            String[] contactCases = {
                "oak_boat", "minecart", "item_frame", "painting", "falling_block",
                "leash_knot", "cushion", "end_crystal", "tnt",
                "armor_stand", "armor_stand_marker", "oak_boat_no_star",
                "armor_stand_blast_nearby"
            };
            for (String name : contactCases) {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                if (name.equals("item_frame") || name.equals("painting")) {
                    for (int y = 64; y <= 66; y++) {
                        for (int z = 2; z <= 6; z++) {
                            level.setBlock(new BlockPos(4, y, z), Blocks.STONE.defaultBlockState(), 2);
                        }
                    }
                }
                if (name.equals("leash_knot")) {
                    level.setBlock(new BlockPos(3, 64, 4), Blocks.OAK_FENCE.defaultBlockState(), 2);
                }
                if (name.equals("armor_stand_blast_nearby")) {
                    for (int y = 64; y <= 66; y++) {
                        for (int z = 3; z <= 5; z++) {
                            level.setBlock(new BlockPos(6, y, z), Blocks.STONE.defaultBlockState(), 2);
                        }
                    }
                }

                Map<String, Entity> targets = new LinkedHashMap<>();
                double spawnY = 64.5;
                switch (name) {
                    case "oak_boat", "oak_boat_no_star" -> {
                        spawnY = 64.3;
                        targets.put("boat", summon(level, "summon minecraft:oak_boat 3.5 64.0 4.5 {}"));
                    }
                    case "minecart" -> {
                        spawnY = 64.4;
                        targets.put("minecart", summon(level, "summon minecraft:minecart 3.5 64.0 4.5 {}"));
                    }
                    case "item_frame" -> {
                        targets.put("frame", summon(level, "summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:\"minecraft:diamond\",count:1}}"));
                    }
                    case "painting" -> {
                        targets.put("painting", summon(level, "summon minecraft:painting 3.5 64.5 4.5 {block_pos:[I;3,64,4],facing:1b,variant:\"minecraft:kebab\"}"));
                    }
                    case "falling_block" -> {
                        spawnY = 65.3;
                        targets.put("falling", summon(level, "summon minecraft:falling_block 3.5 65.0 4.5 {BlockState:\"minecraft:sand\",Time:1,Motion:[0.0d,0.0d,0.0d]}"));
                    }
                    case "leash_knot" -> {
                        spawnY = 64.6;
                        targets.put("knot", summon(level, "summon minecraft:leash_knot 3.5 64.375 4.5 {}"));
                    }
                    case "cushion" -> {
                        spawnY = 64.1;
                        targets.put("cushion", summon(level, "summon minecraft:cushion 3.5 64.0 4.5 {block_pos:[I;3,64,4],color:\"white\"}"));
                    }
                    case "end_crystal" -> {
                        spawnY = 65.0;
                        targets.put("crystal", summon(level, "summon minecraft:end_crystal 4.25 64.0 4.5 {}"));
                    }
                    case "tnt" -> {
                        targets.put("tnt", summon(level, "summon minecraft:tnt 3.5 64.0 4.5 {fuse:80}"));
                    }
                    case "armor_stand" -> {
                        targets.put("stand", summon(level, "summon minecraft:armor_stand 3.5 64.0 4.5 {}"));
                    }
                    case "armor_stand_marker" -> {
                        targets.put("stand", summon(level, "summon minecraft:armor_stand 3.5 64.0 4.5 {Marker:1b}"));
                    }
                    case "armor_stand_blast_nearby" -> {
                        targets.put("normal", summon(level, "summon minecraft:armor_stand 5.5 64.0 4.5 {}"));
                        targets.put("invisible", summon(level, "summon minecraft:armor_stand 3.5 64.0 6.5 {Invisible:1b}"));
                        targets.put("marker", summon(level, "summon minecraft:armor_stand 3.5 64.0 2.5 {Marker:1b}"));
                        targets.put("occluded", summon(level, "summon minecraft:armor_stand 7.5 64.0 4.5 {}"));
                        targets.put("far", summon(level, "summon minecraft:armor_stand 9.5 64.0 4.5 {}"));
                    }
                    default -> throw new IllegalArgumentException(name);
                }

                Set<Integer> br = entityIds(level);
                if (name.equals("armor_stand_blast_nearby")) {
                    cmd("summon minecraft:firework_rocket 3.5 64.5 4.5 {LifeTime:1,ShotAtAngle:1b,Motion:[0.0d,0.0d,0.0d],FireworksItem:" + STAR_FIREWORK_ITEM + "}");
                } else {
                    boolean plain = name.equals("oak_boat_no_star")
                        || name.equals("item_frame")
                        || name.equals("painting")
                        || name.equals("leash_knot");
                    String item = plain ? PLAIN_FIREWORK_ITEM : STAR_FIREWORK_ITEM;
                    cmd("summon minecraft:firework_rocket 1.5 " + spawnY + " 4.5 {LifeTime:20,ShotAtAngle:1b,Motion:[1.0d,0.0d,0.0d],FireworksItem:" + item + "}");
                }
                FireworkRocketEntity rocket = findNew(level, br, FireworkRocketEntity.class);
                seedProj(rocket);

                JsonObject rec = new JsonObject();
                rec.addProperty("case", name);
                rec.addProperty("spawn_y", spawnY);
                JsonArray ticks = new JsonArray();
                for (int t = 1; t <= 8; t++) {
                    tickGameEvents.clear();
                    tickSounds.clear();
                    stepTick();
                    drainChannel();
                    JsonObject row = new JsonObject();
                    row.addProperty("tick", t);
                    row.addProperty("rocket_alive", !rocket.isRemoved());
                    row.add("rocket_pos_bits", vecBits(rocket.position()));
                    row.add("rocket_vel_bits", vecBits(rocket.getDeltaMovement()));
                    JsonObject aliveMap = new JsonObject();
                    for (Map.Entry<String, Entity> entry : targets.entrySet()) {
                        aliveMap.addProperty(entry.getKey(), entry.getValue().isAlive());
                    }
                    row.add("targets_alive", aliveMap);
                    row.add("drops", snapDrops(level));
                    row.add("game_events", snapEvents());
                    row.add("sounds", snapSounds());
                    ticks.add(row);
                }
                rec.add("ticks", ticks);
                out.add(rec);
            }
            return out;
        }

        JsonObject snapReviewTick(ServerLevel level, int t, Map<String, Entity> tracked) {
            JsonObject row = new JsonObject();
            row.addProperty("tick", t);
            row.addProperty("player_health", player.getHealth());
            int bolts = 0;
            int orbs = 0;
            for (Entity e : level.getAllEntities()) {
                if (e instanceof LightningBolt && !e.isRemoved()) bolts++;
                if (BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString().equals("minecraft:experience_orb") && !e.isRemoved()) orbs++;
            }
            row.addProperty("lightning_count", bolts);
            row.addProperty("xp_orb_count", orbs);
            var targetState = level.getBlockState(new BlockPos(4, 64, 4));
            row.addProperty("target_power", targetState.hasProperty(BlockStateProperties.POWER) ? targetState.getValue(BlockStateProperties.POWER) : -1);
            var sculkState = level.getBlockState(new BlockPos(4, 64, 6));
            row.addProperty("sculk_phase", sculkState.hasProperty(SculkSensorBlock.PHASE) ? sculkState.getValue(SculkSensorBlock.PHASE).getSerializedName() : null);

            JsonObject ents = new JsonObject();
            for (Map.Entry<String, Entity> entry : tracked.entrySet()) {
                Entity e = entry.getValue();
                JsonObject eo = new JsonObject();
                eo.addProperty("present", !e.isRemoved());
                eo.addProperty("alive", e instanceof LivingEntity le ? le.isAlive() : !e.isRemoved());
                eo.add("pos_bits", vecBits(e.position()));
                eo.add("vel_bits", vecBits(e.getDeltaMovement()));
                eo.addProperty("yaw_bits", fBits(e.getYRot()));
                eo.addProperty("fire_ticks", e.getRemainingFireTicks());
                eo.addProperty("ticks_frozen", e.getTicksFrozen());
                if (e instanceof LivingEntity le) {
                    eo.addProperty("health", le.getHealth());
                } else {
                    eo.add("health", com.google.gson.JsonNull.INSTANCE);
                }
                if (e instanceof ItemFrame frame) {
                    ItemStack st = frame.getItem();
                    eo.addProperty("frame_item", st.isEmpty() ? null : BuiltInRegistries.ITEM.getKey(st.getItem()).toString());
                } else {
                    eo.add("frame_item", com.google.gson.JsonNull.INSTANCE);
                }
                if (e instanceof Pig pig) {
                    eo.addProperty("leashed", pig.isLeashed());
                } else {
                    eo.add("leashed", com.google.gson.JsonNull.INSTANCE);
                }
                ents.add(entry.getKey(), eo);
            }
            row.add("entities", ents);
            row.add("drops", snapDrops(level));
            row.add("game_events", snapEvents());
            row.add("sounds", snapSounds());
            return row;
        }

        JsonObject recordReviewTicks(ServerLevel level, String name, int nTicks, Map<String, Entity> tracked) {
            JsonObject rec = new JsonObject();
            rec.addProperty("case", name);
            JsonArray ticks = new JsonArray();
            for (int t = 1; t <= nTicks; t++) {
                tickGameEvents.clear();
                tickSounds.clear();
                stepTick();
                drainChannel();
                ticks.add(snapReviewTick(level, t, tracked));
            }
            rec.add("ticks", ticks);
            return rec;
        }

        // Round-1 review regressions (findings #1-#27) in real vanilla 26.3.
        JsonArray runReviewRound1(ServerLevel level) throws Exception {
            JsonArray out = new JsonArray();
            String owner = playerUuidNbt();

            // Finding #1a: Splash potion blocked by bottom stone slab before pig behind it.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                cmd("setblock 3 64 4 minecraft:stone_slab[type=bottom]");
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("pig", summon(level, "summon minecraft:pig 8.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
                tr.put("potion", summon(level, "summon minecraft:splash_potion 1.5 64.3 4.5 {Motion:[1.0d,0.0d,0.0d],Item:{id:\"minecraft:splash_potion\",count:1,components:{\"minecraft:potion_contents\":{potion:\"minecraft:harming\"}}}}"));
                out.add(recordReviewTicks(level, "r01_potion_slab_blocks_pig", 4, tr));
            }

            // Finding #1b: Experience bottle blocked by bottom stone slab at exact collider clip.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                cmd("setblock 3 64 4 minecraft:stone_slab[type=bottom]");
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("bottle", summon(level, "summon minecraft:experience_bottle 1.5 64.3 4.5 {Motion:[1.0d,0.0d,0.0d]}"));
                out.add(recordReviewTicks(level, "r01_exp_bottle_slab_clip", 4, tr));
            }

            // Finding #1c: Fast splash potion hits thin item frame instead of skipping between samples.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                for (int y = 64; y <= 66; y++) {
                    for (int z = 2; z <= 6; z++) {
                        level.setBlock(new BlockPos(4, y, z), Blocks.STONE.defaultBlockState(), 2);
                    }
                }
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("frame", summon(level, "summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:\"minecraft:diamond\",count:1}}"));
                tr.put("potion", summon(level, "summon minecraft:splash_potion 1.5 64.5 4.5 {Motion:[3.0d,0.0d,0.0d],Item:{id:\"minecraft:splash_potion\",count:1,components:{\"minecraft:potion_contents\":{potion:\"minecraft:water\"}}}}"));
                out.add(recordReviewTicks(level, "r01_potion_thin_frame", 3, tr));
            }

            // Finding #2a: Two arrows in same tick aimed at an empty item frame; second flies past into pig.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                for (int x = 2; x <= 5; x++) {
                    for (int y = 64; y <= 66; y++) {
                        level.setBlock(new BlockPos(x, y, 5), Blocks.STONE.defaultBlockState(), 2);
                    }
                }
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("frame", summon(level, "summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:2b}"));
                tr.put("pig", summon(level, "summon minecraft:pig 5.5 64.0 4.54 {NoAI:1b,Silent:1b,Health:10.0f}"));
                tr.put("arrow1", summon(level, "summon minecraft:arrow 1.5 64.5 4.96 {Motion:[2.5d,0.0d,0.0d],damage:2.0d}"));
                tr.put("arrow2", summon(level, "summon minecraft:arrow 1.5 64.5 4.96 {Motion:[2.5d,0.0d,0.0d],damage:2.0d}"));
                out.add(recordReviewTicks(level, "r02_two_arrows_empty_frame", 4, tr));
            }

            // Finding #2b: Expiring rocket breaks armor stand before arrow summoned after it ticks in same tick.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("stand", summon(level, "summon minecraft:armor_stand 4.5 64.0 4.5 {}"));
                tr.put("pig", summon(level, "summon minecraft:pig 7.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:10.0f}"));
                tr.put("rocket", summon(level, "summon minecraft:firework_rocket 3.5 64.5 4.5 {LifeTime:0,ShotAtAngle:1b,Motion:[0.0d,0.0d,0.0d],FireworksItem:" + STAR_FIREWORK_ITEM + "}"));
                tr.put("arrow", summon(level, "summon minecraft:arrow 2.5 64.5 4.5 {Motion:[3.0d,0.0d,0.0d],damage:2.0d}"));
                out.add(recordReviewTicks(level, "r02_rocket_breaks_stand_before_arrow", 4, tr));
            }

            // Finding #3: Piercing arrow hits two End crystals in same tick; removing crystal 1 shifts storage before crystal 2.
            {
                clearArena(level, 16);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("crystal1", summon(level, "summon minecraft:end_crystal 3.5 64.0 4.5 {}"));
                tr.put("crystal2", summon(level, "summon minecraft:end_crystal 10.5 64.0 4.5 {Invulnerable:1b}"));
                tr.put("arrow", summon(level, "summon minecraft:arrow 1.5 65.0 4.5 {Motion:[9.5d,0.0d,0.0d],PierceLevel:2b,damage:2.0d}"));
                out.add(recordReviewTicks(level, "r03_pierce_two_crystals", 3, tr));
            }

            // Finding #4a: Equal-distance tie-break follows section encounter order (stand first vs parrot first, both width 0.5).
            for (boolean standFirst : new boolean[]{true, false}) {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                if (standFirst) {
                    tr.put("stand", summon(level, "summon minecraft:armor_stand 3.5 64.0 4.5 {}"));
                    tr.put("parrot", summon(level, "summon minecraft:parrot 3.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:6.0f}"));
                } else {
                    tr.put("parrot", summon(level, "summon minecraft:parrot 3.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:6.0f}"));
                    tr.put("stand", summon(level, "summon minecraft:armor_stand 3.5 64.0 4.5 {}"));
                }
                tr.put("snowball", summon(level, "summon minecraft:snowball 1.5 64.3 4.5 {Motion:[2.0d,0.0d,0.0d]}"));
                out.add(recordReviewTicks(level, standFirst ? "r04_tie_stand_before_parrot" : "r04_tie_parrot_before_stand", 3, tr));
            }

            // Finding #4b: Arrow batch sorting uses HangingEntity center position(), not box bottom.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                for (int y = 64; y <= 66; y++) {
                    for (int z = 2; z <= 6; z++) {
                        level.setBlock(new BlockPos(4, y, z), Blocks.STONE.defaultBlockState(), 2);
                    }
                }
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("stand", summon(level, "summon minecraft:armor_stand 3.74 64.0 4.5 {}"));
                tr.put("frame", summon(level, "summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:\"minecraft:diamond\",count:1}}"));
                tr.put("arrow", summon(level, "summon minecraft:arrow 3.0 64.85 4.5 {Motion:[1.5d,0.0d,0.0d],damage:2.0d}"));
                out.add(recordReviewTicks(level, "r04_arrow_sort_frame_center", 3, tr));
            }

            // Finding #5a: Wind charge redirects ghast fireball (#redirectable_projectile) instead of passing through.
            {
                clearArena(level, 12);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("fireball", summon(level, "summon minecraft:fireball 3.5 65.0 4.5 {ExplosionPower:1b,acceleration_power:0.0d,Motion:[0.0d,0.0d,0.0d]}"));
                tr.put("wind", summon(level, "summon minecraft:wind_charge 1.5 65.0 4.5 {acceleration_power:0.0d,Motion:[1.0d,0.0d,0.0d],Owner:" + owner + ",LeftOwner:1b}"));
                out.add(recordReviewTicks(level, "r05_wind_charge_redirects_fireball", 4, tr));
            }

            // Finding #5b: Arrow hits and destroys a pickable ShulkerBullet.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("bullet", summon(level, "summon minecraft:shulker_bullet 3.5 65.0 4.5 {Motion:[0.0d,0.0d,0.0d]}"));
                tr.put("arrow", summon(level, "summon minecraft:arrow 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],damage:2.0d}"));
                out.add(recordReviewTicks(level, "r05_arrow_pops_shulker_bullet", 4, tr));
            }

            // Finding #5c: Wind charges pass through each other (`AbstractWindCharge.canHitEntity`).
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("wind1", summon(level, "summon minecraft:wind_charge 1.5 65.0 4.5 {acceleration_power:0.0d,Motion:[1.0d,0.0d,0.0d]}"));
                tr.put("wind2", summon(level, "summon minecraft:wind_charge 4.5 65.0 4.5 {acceleration_power:0.0d,Motion:[-1.0d,0.0d,0.0d]}"));
                out.add(recordReviewTicks(level, "r05_wind_charge_skips_wind_charge", 4, tr));
            }

            // Finding #6: Owner's root-vehicle tree excludes boat & co-passenger while !leftOwner.
            {
                clearArena(level, 12);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                AbstractBoat boat = (AbstractBoat) summon(level, "summon minecraft:oak_boat 2.5 64.0 4.5 {}");
                Pig copass = (Pig) summon(level, "summon minecraft:pig 2.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:10.0f}");
                player.startRiding(boat, true, false);
                copass.startRiding(boat, true, false);
                Pig outside = (Pig) summon(level, "summon minecraft:pig 6.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:10.0f}");
                stepTick();
                drainChannel();
                tr.put("boat", boat);
                tr.put("copassenger", copass);
                tr.put("outside_pig", outside);
                tr.put("arrow", summon(level, "summon minecraft:arrow 2.5 64.8 4.5 {Motion:[2.0d,0.0d,0.0d],damage:2.0d,Owner:" + owner + ",LeftOwner:0b}"));
                out.add(recordReviewTicks(level, "r06_arrow_skips_boat_copassenger", 5, tr));
            }

            // Finding #7a: Firework rocket tick-8 computeMargin f32 boundary.
            {
                clearArena(level, 12);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("stand", summon(level, "summon minecraft:armor_stand 5.7 64.0 5.050000005 {NoGravity:1b}"));
                tr.put("rocket", summon(level, "summon minecraft:firework_rocket 1.5 64.5 4.5 {LifeTime:20,ShotAtAngle:1b,Motion:[0.5d,0.0d,0.0d],FireworksItem:" + STAR_FIREWORK_ITEM + "}"));
                out.add(recordReviewTicks(level, "r07_firework_margin_tick8", 10, tr));
            }

            // Finding #7b: Trident wall endpoint rejects entity touching x=4.0.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                level.setBlock(new BlockPos(4, 64, 4), Blocks.STONE.defaultBlockState(), 2);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("stand", summon(level, "summon minecraft:armor_stand 4.25 64.0 4.75 {NoGravity:1b}"));
                tr.put("trident", summon(level, "summon minecraft:trident 3.5 64.5 4.5 {Motion:[1.0d,0.0d,0.0d]}"));
                out.add(recordReviewTicks(level, "r07_trident_wall_endpoint", 3, tr));
            }

            // Finding #9: Pierce 0 arrow batch continues past refused falling block to hit zombie behind it.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("falling", summon(level, "summon minecraft:falling_block 2.5 65.0 4.5 {BlockState:\"minecraft:sand\",Time:1,Motion:[0.0d,0.0d,0.0d]}"));
                tr.put("zombie", summon(level, "summon minecraft:zombie 3.8 64.0 4.5 {NoAI:1b,Silent:1b,Health:20.0f}"));
                tr.put("arrow", summon(level, "summon minecraft:arrow 1.5 65.3 4.5 {Motion:[3.0d,0.0d,0.0d],damage:2.0d,PierceLevel:0b}"));
                out.add(recordReviewTicks(level, "r09_pierce0_falling_block_then_zombie", 4, tr));
            }

            // Finding #10: Piercing arrow hits pig then Survival player in the same tick.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                player.snapTo(3.8, 64.0, 4.5, -90.0f, 0.0f);
                stepTick();
                drainChannel();
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("pig", summon(level, "summon minecraft:pig 2.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:10.0f}"));
                tr.put("arrow", summon(level, "summon minecraft:arrow 1.5 64.5 4.5 {Motion:[3.0d,0.0d,0.0d],damage:2.0d,PierceLevel:2b}"));
                out.add(recordReviewTicks(level, "r10_pierce_pig_then_player", 3, tr));
            }

            // Finding #11: Friendly-fire (`pvp false` and `team friendlyFire false`) pre-selection filter for arrow & trident.
            for (String variant : new String[]{"pvp_false_arrow", "pvp_false_trident", "team_ff_false_trident"}) {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                if (variant.startsWith("pvp_false")) {
                    cmd("gamerule pvp false");
                } else {
                    cmd("team add blue");
                    cmd("team modify blue friendlyFire false");
                    cmd("team join blue Player");
                }
                player.snapTo(3.5, 64.0, 4.5, -90.0f, 0.0f);
                stepTick();
                drainChannel();
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("pig", summon(level, "summon minecraft:pig 6.8 64.0 4.5 {NoAI:1b,Silent:1b,Health:10.0f}"));
                if (variant.endsWith("_arrow")) {
                    tr.put("proj", summon(level, "summon minecraft:arrow 1.5 64.5 4.5 {Motion:[1.5d,0.0d,0.0d],damage:2.0d,Owner:" + owner + ",LeftOwner:1b}"));
                } else {
                    tr.put("proj", summon(level, "summon minecraft:trident 1.5 64.5 4.5 {Motion:[1.5d,0.0d,0.0d],Owner:" + owner + ",LeftOwner:1b}"));
                }
                out.add(recordReviewTicks(level, "r11_" + variant, 6, tr));
            }

            // Finding #12: Piercing 1 critical arrow hitting 3 zombies (crit RNG after pierce check, 3rd skipped).
            {
                clearArena(level, 11);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("mob0", summon(level, "summon minecraft:zombie 2.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:20.0f}"));
                tr.put("mob1", summon(level, "summon minecraft:zombie 3.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:20.0f}"));
                tr.put("mob2", summon(level, "summon minecraft:zombie 4.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:20.0f}"));
                tr.put("arrow", summon(level, "summon minecraft:arrow 1.5 64.5 4.5 {Motion:[4.0d,0.0d,0.0d],damage:2.0d,crit:1b,PierceLevel:1b}"));
                out.add(recordReviewTicks(level, "r12_pierce1_crit_three_zombies", 3, tr));
            }

            // Finding #14a: Breeze deflects firework rocket (REVERSE RNG + yaw + sound, does not explode).
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("breeze", summon(level, "summon minecraft:breeze 3.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
                tr.put("rocket", summon(level, "summon minecraft:firework_rocket 1.5 64.5 4.5 {LifeTime:20,ShotAtAngle:1b,Motion:[1.0d,0.0d,0.0d],FireworksItem:" + STAR_FIREWORK_ITEM + "}"));
                out.add(recordReviewTicks(level, "r14_breeze_deflects_firework", 5, tr));
            }

            // Finding #14b: Breeze deflects snowball (REVERSE RNG + yaw + sound, stays alive).
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("breeze", summon(level, "summon minecraft:breeze 3.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
                tr.put("snowball", summon(level, "summon minecraft:snowball 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d]}"));
                out.add(recordReviewTicks(level, "r14_breeze_deflects_snowball", 5, tr));
            }

            // Finding #14c: Same Breeze does not deflect a slow arrow twice (`lastDeflectedBy`).
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("breeze", summon(level, "summon minecraft:breeze 3.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
                tr.put("arrow", summon(level, "summon minecraft:arrow 3.1 65.0 4.5 {Motion:[0.2d,0.0d,0.0d],NoGravity:1b}"));
                out.add(recordReviewTicks(level, "r14_breeze_arrow_no_repeat", 5, tr));
            }

            // Finding #15: Trident singleton sweep prefers closer box entry (crystal) over closer center (chicken).
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("chicken", summon(level, "summon minecraft:chicken 3.6 64.5 4.5 {NoAI:1b,Silent:1b,NoGravity:1b,Health:4.0f}"));
                tr.put("crystal", summon(level, "summon minecraft:end_crystal 4.2 64.0 4.5 {}"));
                tr.put("trident", summon(level, "summon minecraft:trident 1.5 65.0 4.5 {Motion:[3.0d,0.0d,0.0d]}"));
                out.add(recordReviewTicks(level, "r15_trident_singleton_box_entry", 3, tr));
            }

            // Finding #16: Channeling trident hitting an armor stand during thunder summons lightning.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                cmd("weather thunder 1000");
                level.setRainLevel(1.0f);
                level.setThunderLevel(1.0f);
                stepTick();
                drainChannel();
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("stand", summon(level, "summon minecraft:armor_stand 3.5 64.0 4.5 {}"));
                tr.put("trident", summon(level, "summon minecraft:trident 1.5 64.5 4.5 {Motion:[1.0d,0.0d,0.0d],Owner:" + owner + ",LeftOwner:1b,item:{id:\"minecraft:trident\",count:1,components:{\"minecraft:enchantments\":{\"minecraft:channeling\":1}}}}"));
                out.add(recordReviewTicks(level, "r16_trident_channeling_stand", 5, tr));
            }

            // Finding #17: Invulnerable Enderman in boat refuses flaming arrow without teleporting or bouncing arrow.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                Set<Integer> bt = entityIds(level);
                cmd("summon minecraft:oak_boat 3.5 64.0 4.5 {Passengers:[{id:\"minecraft:enderman\",NoAI:1b,Silent:1b,Invulnerable:1b}]}");
                Entity enderman = findNew(level, bt, LivingEntity.class);
                seedProj(enderman);
                tr.put("enderman", enderman);
                tr.put("arrow", summon(level, "summon minecraft:arrow 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],Fire:100s}"));
                out.add(recordReviewTicks(level, "r17_invuln_enderman_in_boat_flame_arrow", 5, tr));
            }

            // Finding #18: Arrow Punch knockback horizontal normalization threshold (5e-6 vs 2e-5).
            for (boolean sub : new boolean[]{true, false}) {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("pig", summon(level, "summon minecraft:pig 3.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:20.0f}"));
                String vx = sub ? "5.0e-6d" : "2.0e-5d";
                tr.put("arrow", summon(level, "summon minecraft:arrow 3.5 65.5 4.5 {Motion:[" + vx + ",-1.0d,0.0d],damage:1.0d,weapon:{id:\"minecraft:bow\",count:1,components:{\"minecraft:enchantments\":{\"minecraft:punch\":1}}}}"));
                out.add(recordReviewTicks(level, sub ? "r18_arrow_knockback_subthreshold" : "r18_arrow_knockback_above_threshold", 3, tr));
            }

            // Finding #19: Special target rules (unowned LlamaSpit skips crystal; owned LlamaSpit and ShulkerBullet break crystal).
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("crystal", summon(level, "summon minecraft:end_crystal 4.25 64.0 4.5 {}"));
                tr.put("spit", summon(level, "summon minecraft:llama_spit 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d]}"));
                out.add(recordReviewTicks(level, "r19_unowned_llama_spit_crystal", 5, tr));
            }
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                Entity llama = summon(level, "summon minecraft:llama 0.5 64.0 20.5 {NoAI:1b,NoGravity:1b,Silent:1b}");
                tr.put("crystal", summon(level, "summon minecraft:end_crystal 4.25 64.0 4.5 {}"));
                tr.put("spit", summon(level, "summon minecraft:llama_spit 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],Owner:" + uuidNbt(llama) + ",LeftOwner:1b}"));
                out.add(recordReviewTicks(level, "r19_owned_llama_spit_crystal", 5, tr));
            }
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("crystal", summon(level, "summon minecraft:end_crystal 4.25 64.0 4.5 {}"));
                tr.put("bullet", summon(level, "summon minecraft:shulker_bullet 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d]}"));
                out.add(recordReviewTicks(level, "r19_shulker_bullet_crystal", 5, tr));
            }

            // Finding #20a: Arrow kills armor stand (clearing equipment before post-attack Thorns runs).
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("stand", summon(level, "summon minecraft:armor_stand 3.5 64.0 4.5 {equipment:{chest:{id:\"minecraft:iron_chestplate\",count:1,components:{\"minecraft:enchantments\":{\"minecraft:thorns\":3}}}}}"));
                tr.put("arrow", summon(level, "summon minecraft:arrow 1.5 64.5 4.5 {Motion:[1.0d,0.0d,0.0d],damage:2.0d,Owner:" + owner + ",LeftOwner:1b}"));
                out.add(recordReviewTicks(level, "r20_thorns_armor_stand_arrow", 4, tr));
            }
            // Finding #20b: Large fireball runs doPostAttackEffects on invulnerable stand even when hurtServer refuses (Thorns hurts owner).
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                String thornsPiece = "{id:\"minecraft:iron_chestplate\",count:1,components:{\"minecraft:enchantments\":{\"minecraft:thorns\":3}}}";
                tr.put("stand", summon(level, "summon minecraft:armor_stand 3.5 64.0 4.5 {Invulnerable:1b,equipment:{head:{id:\"minecraft:iron_helmet\",count:1,components:{\"minecraft:enchantments\":{\"minecraft:thorns\":3}}},chest:" + thornsPiece + ",legs:{id:\"minecraft:iron_leggings\",count:1,components:{\"minecraft:enchantments\":{\"minecraft:thorns\":3}}},feet:{id:\"minecraft:iron_boots\",count:1,components:{\"minecraft:enchantments\":{\"minecraft:thorns\":3}}}}}"));
                tr.put("fireball", summon(level, "summon minecraft:fireball 1.5 64.5 4.5 {ExplosionPower:1b,acceleration_power:0.0d,Motion:[1.0d,0.0d,0.0d],Owner:" + owner + ",LeftOwner:1b}"));
                out.add(recordReviewTicks(level, "r20_large_fireball_thorns_stand", 4, tr));
            }

            // Finding #21a: Flame arrow hitting frozen invulnerable stand clears TicksFrozen while restoring fire_ticks.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("stand", summon(level, "summon minecraft:armor_stand 3.5 64.0 4.5 {Invulnerable:1b,TicksFrozen:140}"));
                tr.put("arrow", summon(level, "summon minecraft:arrow 1.5 64.5 4.5 {Motion:[1.0d,0.0d,0.0d],Fire:100s}"));
                out.add(recordReviewTicks(level, "r21_frozen_stand_flame_arrow", 4, tr));
            }

            // Finding #21b: Visible armor stand in fire with Fire:-2s still runs in_fire hurt and ignites to 100.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                level.setBlock(new BlockPos(3, 64, 4), Blocks.FIRE.defaultBlockState(), 2);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("stand", summon(level, "summon minecraft:armor_stand 3.5 64.0 4.5 {Fire:-2s}"));
                out.add(recordReviewTicks(level, "r21_stand_in_fire_neg2", 3, tr));
            }

            // Finding #23: Replacement leash knot on same fence does not inherit old pig's broken lead.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                level.setBlock(new BlockPos(3, 64, 4), Blocks.OAK_FENCE.defaultBlockState(), 2);
                Pig pigA = (Pig) summon(level, "summon minecraft:pig 3.5 64.0 6.5 {NoAI:1b,Silent:1b,leash:[I;3,64,4]}");
                stepTick();
                drainChannel();
                Entity arrow = summon(level, "summon minecraft:arrow 1.5 64.6 4.5 {Motion:[2.0d,0.0d,0.0d]}");
                // Tick 1: arrow breaks knot #1. Before tick 2, summon pigB tied to same fence to create knot #2!
                tickGameEvents.clear();
                tickSounds.clear();
                stepTick();
                drainChannel();
                Pig pigB = (Pig) summon(level, "summon minecraft:pig 3.5 64.0 2.5 {NoAI:1b,Silent:1b,leash:[I;3,64,4]}");
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("pig_a", pigA);
                tr.put("pig_b", pigB);
                tr.put("arrow", arrow);
                out.add(recordReviewTicks(level, "r23_replacement_knot_drops_old_lead", 3, tr));
            }

            // Finding #25a: Firework rocket hitting boat on tick 1 (life == 0) still plays launch sound.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("boat", summon(level, "summon minecraft:oak_boat 3.5 64.0 4.5 {}"));
                tr.put("rocket", summon(level, "summon minecraft:firework_rocket 2.5 64.3 4.5 {LifeTime:20,ShotAtAngle:1b,Motion:[1.0d,0.0d,0.0d],FireworksItem:" + STAR_FIREWORK_ITEM + "}"));
                out.add(recordReviewTicks(level, "r25_rocket_first_tick_boat_sound", 2, tr));
            }

            // Finding #25b: Firework rocket hitting target block triggers redstone power and block-center PROJECTILE_LAND.
            {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                level.setBlock(new BlockPos(4, 64, 4), Blocks.TARGET.defaultBlockState(), 2);
                Map<String, Entity> tr = new LinkedHashMap<>();
                tr.put("rocket", summon(level, "summon minecraft:firework_rocket 2.5 64.5 4.5 {LifeTime:20,ShotAtAngle:1b,Motion:[2.0d,0.0d,0.0d],FireworksItem:" + PLAIN_FIREWORK_ITEM + "}"));
                out.add(recordReviewTicks(level, "r25_rocket_hits_target_block", 3, tr));
            }

            // Finding #26: Firework explosion damages stand & pig in section order (both summon orders).
            for (boolean standFirst : new boolean[]{true, false}) {
                clearArena(level, 10);
                level.getRandom().setSeed(LEVEL_SEED);
                Map<String, Entity> tr = new LinkedHashMap<>();
                if (standFirst) {
                    tr.put("stand", summon(level, "summon minecraft:armor_stand 5.5 64.0 4.5 {}"));
                    tr.put("pig", summon(level, "summon minecraft:pig 5.5 64.0 5.5 {NoAI:1b,Silent:1b,Health:20.0f}"));
                } else {
                    tr.put("pig", summon(level, "summon minecraft:pig 5.5 64.0 5.5 {NoAI:1b,Silent:1b,Health:20.0f}"));
                    tr.put("stand", summon(level, "summon minecraft:armor_stand 5.5 64.0 4.5 {}"));
                }
                tr.put("rocket", summon(level, "summon minecraft:firework_rocket 3.5 64.5 4.5 {LifeTime:1,ShotAtAngle:1b,Motion:[0.0d,0.0d,0.0d],FireworksItem:" + STAR_FIREWORK_ITEM + "}"));
                out.add(recordReviewTicks(level, standFirst ? "r26_rocket_blast_stand_and_mob" : "r26_rocket_blast_mob_then_stand", 3, tr));
            }

            // Finding #27: Block PROJECTILE_LAND at exact block center (4.5,64.5,4.5) and wool dampening on sculk sensor.
            for (String blockKind : new String[]{"stone", "wool"}) {
                for (String projKind : new String[]{"arrow", "trident", "snowball", "wind_charge", "small_fireball", "wither_skull", "splash_potion", "firework_rocket"}) {
                    clearArena(level, 10);
                    level.getRandom().setSeed(LEVEL_SEED);
                    cmd("setblock 4 64 4 " + (blockKind.equals("wool") ? "minecraft:white_wool" : "minecraft:stone"));
                    level.setBlock(new BlockPos(4, 64, 6), Blocks.SCULK_SENSOR.defaultBlockState(), 2);
                    stepTick();
                    drainChannel();
                    Map<String, Entity> tr = new LinkedHashMap<>();
                    String cmdStr = switch (projKind) {
                        case "arrow" -> "summon minecraft:arrow 2.5 64.75 4.5 {Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b}";
                        case "trident" -> "summon minecraft:trident 2.5 64.75 4.5 {Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b}";
                        case "snowball" -> "summon minecraft:snowball 2.5 64.75 4.5 {Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b}";
                        case "wind_charge" -> "summon minecraft:wind_charge 2.5 64.75 4.5 {acceleration_power:0.0d,Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b}";
                        case "small_fireball" -> "summon minecraft:small_fireball 2.5 64.75 4.5 {acceleration_power:0.0d,Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b}";
                        case "wither_skull" -> "summon minecraft:wither_skull 2.5 64.75 4.5 {acceleration_power:0.0d,Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b}";
                        case "splash_potion" -> "summon minecraft:splash_potion 2.5 64.75 4.5 {Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b,Item:{id:\"minecraft:splash_potion\",count:1,components:{\"minecraft:potion_contents\":{potion:\"minecraft:water\"}}}}";
                        case "firework_rocket" -> "summon minecraft:firework_rocket 2.5 64.75 4.5 {LifeTime:20,ShotAtAngle:1b,Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b,FireworksItem:" + PLAIN_FIREWORK_ITEM + "}";
                        default -> throw new IllegalArgumentException(projKind);
                    };
                    tr.put("proj", summon(level, cmdStr));
                    out.add(recordReviewTicks(level, "r27_" + blockKind + "_" + projKind, 6, tr));
                }
            }

            return out;
        }

        Entity summon(ServerLevel level, String command) throws Exception {
            Set<Integer> before = entityIds(level);
            cmd(command);
            Entity e = findNew(level, before, Entity.class);
            if (e == null) throw new IllegalStateException("summon failed: " + command);
            seedProj(e);
            return e;
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
            universe = Files.createTempDirectory("sunrise-projectiles-parity-oracle-");
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
