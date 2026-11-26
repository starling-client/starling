package dev.sunriselab.parity;

import com.mojang.authlib.GameProfile;
import com.mojang.serialization.Lifecycle;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.net.Proxy;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Optional;
import java.util.UUID;
import java.util.function.BooleanSupplier;
import net.minecraft.SharedConstants;
import net.minecraft.SystemReport;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.Registries;
import net.minecraft.gametest.framework.GameTestServer;
import net.minecraft.gizmos.GizmoCollector;
import net.minecraft.gizmos.Gizmos;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.Services;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.WorldStem;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.progress.LoggingLevelLoadListener;
import net.minecraft.server.notifications.EmptyNotificationService;
import net.minecraft.server.notifications.NotificationManager;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.permissions.PermissionSet;
import net.minecraft.server.players.NameAndId;
import net.minecraft.server.players.PlayerList;
import net.minecraft.util.Util;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.util.debugchart.LocalSampleLogger;
import net.minecraft.util.debugchart.SampleLogger;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntitySpawnRequest;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.animal.equine.Donkey;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.DataPackConfig;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.dimension.LevelStem;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.WorldDimensions;
import net.minecraft.world.level.levelgen.WorldGenSettings;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.storage.LevelDataAndDimensions;
import net.minecraft.world.level.storage.LevelStorageSource;
import net.minecraft.world.level.storage.PrimaryLevelData;
import net.minecraft.world.phys.Vec3;

// Vanilla 26.3 oracle for ridden NoAI donkey travel and rotation on a stone floor.
public final class VanillaNoAiRiddenTravelOracle {
    static PrintWriter out;
    static Path universe;

    record Case(
        String name,
        boolean saddled,
        boolean noAi,
        boolean clientAuth,
        double initVx,
        double initVy,
        double initVz,
        float forward,
        float strafe,
        float riderYaw,
        float riderPitch,
        int ticks
    ) {}

    static final List<Case> CASES = List.of(
        new Case("forward", true, true, false, 0.0, 0.0, 0.0, 1.0F, 0.0F, 0.0F, 20.0F, 5),
        new Case("turn_and_strafe", true, true, false, 0.0, 0.0, 0.0, 1.0F, 0.5F, 37.5F, -15.0F, 5),
        new Case("backward", true, true, false, 0.0, 0.0, 0.0, -1.0F, 0.0F, 90.0F, 10.0F, 4),
        new Case("client_authoritative", true, true, true, 0.2, 0.0, 0.3, 1.0F, 0.0F, 90.0F, 20.0F, 3),
        new Case("unsaddled", false, true, false, 0.0, 0.0, 0.0, 1.0F, 0.0F, 45.0F, 20.0F, 3)
    );

    static final class TestRider extends Player {
        final boolean clientAuth;

        TestRider(Level level, boolean clientAuth) {
            super(level, new GameProfile(new UUID(0x3331L, 0x2603L), "rider"));
            this.clientAuth = clientAuth;
        }

        @Override
        public GameType gameMode() {
            return GameType.SURVIVAL;
        }

        @Override
        public boolean isClientAuthoritative() {
            return this.clientAuth;
        }
    }

    static String d(double v) {
        return String.format(Locale.ROOT, "%016x", Double.doubleToRawLongBits(v));
    }

    static String f(float v) {
        return String.format(Locale.ROOT, "%08x", Float.floatToRawIntBits(v));
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 1 || args.length > 2) {
            throw new IllegalArgumentException("usage: VanillaNoAiRiddenTravelOracle <out.tsv> [world-dir]");
        }
        out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])));
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Util.startTimerHackThread();
        universe = args.length > 1 ? Path.of(args[1]) : Files.createTempDirectory("sunrise-noai-ridden");
        Files.createDirectories(universe);
        LevelStorageSource.LevelStorageAccess access = LevelStorageSource.createDefault(universe).createAccess("world");
        PackRepository packs = ServerPacksSource.createPackRepository(access);
        MinecraftServer.spin(thread -> Server.create(thread, access, packs));
    }

    static final class Server extends MinecraftServer {
        static final FeatureFlagSet FEATURES = FeatureFlags.REGISTRY
            .allFlags()
            .subtract(FeatureFlagSet.of(FeatureFlags.REDSTONE_EXPERIMENTS, FeatureFlags.MINECART_IMPROVEMENTS));
        final LocalSampleLogger sampleLogger = new LocalSampleLogger(4);
        int phase = 0;

        static Server create(Thread thread, LevelStorageSource.LevelStorageAccess access, PackRepository packs) {
            try {
                packs.reload();
                ArrayList<String> enabled = new ArrayList<>(packs.getAvailableIds());
                enabled.remove("vanilla");
                enabled.addFirst("vanilla");
                WorldDataConfiguration config = new WorldDataConfiguration(new DataPackConfig(enabled, List.of()), FEATURES);
                LevelSettings settings = new LevelSettings("NoAI Ridden", GameType.SURVIVAL, LevelSettings.DifficultySettings.DEFAULT, true, config);
                WorldLoader.PackConfig packConfig = new WorldLoader.PackConfig(packs, config, false, true);
                WorldLoader.InitConfig init = new WorldLoader.InitConfig(packConfig, Commands.CommandSelection.DEDICATED, LevelBasedPermissionSet.OWNER);
                WorldStem stem = Util.<WorldStem>blockUntilDone(executor -> WorldLoader.load(init, context -> {
                    Registry<LevelStem> none = new MappedRegistry<>(Registries.LEVEL_STEM, Lifecycle.stable()).freeze();
                    WorldDimensions dims = context.datapackWorldRegistries().lookupOrThrow(Registries.WORLD_PRESET)
                        .getOrThrow(WorldPresets.FLAT).value().createWorldDimensions();
                    WorldDimensions.Complete complete = dims.bake(none);
                    PrimaryLevelData data = new PrimaryLevelData(settings, complete.specialWorldProperty(), complete.lifecycle());
                    return new WorldLoader.DataLoadOutput<>(
                        new LevelDataAndDimensions.WorldDataAndGenSettings(data, new WorldGenSettings(new WorldOptions(0L, false, false), dims)),
                        complete.dimensionsRegistryAccess());
                }, WorldStem::new, Util.backgroundExecutor(), executor)).get();
                GameRules rules = new GameRules(FEATURES);
                rules.set(GameRules.SPAWN_MOBS, false, null);
                rules.set(GameRules.SPAWN_MONSTERS, false, null);
                rules.set(GameRules.SPAWN_PATROLS, false, null);
                rules.set(GameRules.SPAWN_PHANTOMS, false, null);
                rules.set(GameRules.SPAWN_WANDERING_TRADERS, false, null);
                rules.set(GameRules.SPAWN_WARDENS, false, null);
                rules.set(GameRules.ADVANCE_TIME, false, null);
                rules.set(GameRules.ADVANCE_WEATHER, false, null);
                rules.set(GameRules.RAIDS, false, null);
                Field nf = GameTestServer.class.getDeclaredField("NO_SERVICES");
                nf.setAccessible(true);
                return new Server(thread, access, packs, stem, rules, (Services) nf.get(null));
            } catch (Exception e) {
                e.printStackTrace();
                System.exit(2);
                throw new IllegalStateException(e);
            }
        }

        Server(Thread thread, LevelStorageSource.LevelStorageAccess access, PackRepository packs, WorldStem stem, GameRules rules, Services services) {
            super(thread, access, packs, stem, Optional.of(rules), Proxy.NO_PROXY, DataFixers.getDataFixer(), services,
                LoggingLevelLoadListener.forDedicatedServer(), false, new NotificationManager());
        }

        @Override
        protected boolean initServer() {
            this.setPlayerList(new PlayerList(this, this.registries(), this.playerDataStorage, new EmptyNotificationService()) {});
            Gizmos.withCollector(GizmoCollector.NOOP);
            this.loadLevel();
            return true;
        }

        @Override
        protected void tickServer(BooleanSupplier haveTime) {
            try {
                step(haveTime);
            } catch (Throwable t) {
                t.printStackTrace();
                out.flush();
                System.exit(3);
            }
        }

        void step(BooleanSupplier haveTime) throws Exception {
            ServerLevel level = this.overworld();
            if (phase == 0) {
                level.setChunkForced(0, 0, true);
                phase = 1;
                super.tickServer(haveTime);
                return;
            }
            if (phase == 1) {
                super.tickServer(haveTime);
                if (!level.isPositionEntityTicking(new BlockPos(8, 64, 8))) {
                    return;
                }
                for (int x = 0; x < 16; x++) {
                    for (int z = 0; z < 16; z++) {
                        level.setBlock(new BlockPos(x, 63, z), Blocks.STONE.defaultBlockState(), 2 | 16);
                        for (int y = 64; y <= 70; y++) {
                            level.setBlock(new BlockPos(x, y, z), Blocks.AIR.defaultBlockState(), 2 | 16);
                        }
                    }
                }
                recordCases(level);
                out.close();
                try (var paths = Files.walk(universe)) {
                    paths.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
                } catch (Exception ignored) {
                }
                Runtime.getRuntime().halt(0);
            }
        }

        void recordCases(ServerLevel level) {
            String version = SharedConstants.getCurrentVersion().name();
            out.println("# vanilla " + version + " ridden NoAI donkey travel witness (PR #3331)");
            out.println("# recorded by tools/test_noai_ridden_travel_parity.py --record via VanillaNoAiRiddenTravelOracle");
            out.println("# case\tsaddled\tno_ai\tclient_auth\tinit_vx\tinit_vy\tinit_vz\tforward\tstrafe\trider_yaw\trider_pitch\ttick\tpx\tpy\tpz\tvx\tvy\tvz\tyaw\tpitch\tbody_yaw\thead_yaw\ton_ground");
            for (Case c : CASES) {
                Donkey donkey = EntityTypes.DONKEY.create(level, new EntitySpawnRequest(EntitySpawnReason.COMMAND, true));
                if (donkey == null) {
                    throw new IllegalStateException("failed to create donkey");
                }
                donkey.snapTo(8.5, 64.0, 8.5, 0.0F, 0.0F);
                donkey.setYHeadRot(0.0F);
                donkey.yHeadRotO = 0.0F;
                donkey.setYBodyRot(0.0F);
                donkey.yBodyRotO = 0.0F;
                donkey.setNoAi(c.noAi);
                donkey.setPersistenceRequired();
                donkey.setTamed(true);
                if (c.saddled) {
                    donkey.setItemSlot(EquipmentSlot.SADDLE, new ItemStack(Items.SADDLE));
                }
                if (donkey.isSaddled() != c.saddled) {
                    throw new AssertionError("saddle mismatch for " + c.name);
                }
                donkey.setDeltaMovement(c.initVx, c.initVy, c.initVz);
                donkey.getRandom().setSeed(0L);
                level.addFreshEntity(donkey);
                TestRider rider = new TestRider(level, c.clientAuth);
                rider.snapTo(8.5, 64.0, 8.5, c.riderYaw, c.riderPitch);
                if (!rider.startRiding(donkey, true, false)) {
                    throw new IllegalStateException("could not mount donkey for " + c.name);
                }
                boolean expectedControl = c.saddled;
                if ((donkey.getControllingPassenger() == rider) != expectedControl) {
                    throw new AssertionError("controlling passenger mismatch for " + c.name);
                }
                if (c.saddled && donkey.canSimulateMovement() != !c.clientAuth) {
                    throw new AssertionError("canSimulateMovement mismatch for " + c.name);
                }
                for (int t = 1; t <= c.ticks; t++) {
                    rider.setYRot(c.riderYaw);
                    rider.setXRot(c.riderPitch);
                    rider.yHeadRot = c.riderYaw;
                    rider.yBodyRot = c.riderYaw;
                    rider.xxa = c.strafe;
                    rider.zza = c.forward;
                    donkey.commonTick();
                    donkey.tick();
                    Vec3 v = donkey.getDeltaMovement();
                    out.printf(
                        Locale.ROOT,
                        "%s\t%d\t%d\t%d\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%d\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%d%n",
                        c.name,
                        c.saddled ? 1 : 0,
                        c.noAi ? 1 : 0,
                        c.clientAuth ? 1 : 0,
                        d(c.initVx),
                        d(c.initVy),
                        d(c.initVz),
                        f(c.forward),
                        f(c.strafe),
                        f(c.riderYaw),
                        f(c.riderPitch),
                        t,
                        d(donkey.getX()),
                        d(donkey.getY()),
                        d(donkey.getZ()),
                        d(v.x),
                        d(v.y),
                        d(v.z),
                        f(donkey.getYRot()),
                        f(donkey.getXRot()),
                        f(donkey.yBodyRot),
                        f(donkey.getYHeadRot()),
                        donkey.onGround() ? 1 : 0
                    );
                }
                rider.stopRiding();
                donkey.discard();
            }
            out.flush();
        }

        @Override protected SampleLogger getTickTimeLogger() { return sampleLogger; }
        @Override public boolean isTickTimeLoggingEnabled() { return false; }
        @Override protected void waitUntilNextTick() { this.runAllTasks(); }
        @Override public SystemReport fillServerSystemReport(SystemReport report) { report.setDetail("Type", "NoAI ridden travel oracle"); return report; }
        @Override protected void onServerExit() { super.onServerExit(); System.exit(0); }
        @Override protected void onServerCrash(net.minecraft.CrashReport report) { super.onServerCrash(report); System.exit(1); }
        @Override public boolean isHardcore() { return false; }
        @Override public LevelBasedPermissionSet operatorUserPermissions() { return LevelBasedPermissionSet.ALL; }
        @Override public PermissionSet getFunctionCompilationPermissions() { return LevelBasedPermissionSet.OWNER; }
        @Override public boolean shouldRconBroadcast() { return false; }
        @Override public boolean isDedicatedServer() { return false; }
        @Override public int getRateLimitPacketsPerSecond() { return 0; }
        @Override public int getCommandSpamThresholdSeconds() { return 0; }
        @Override public int getChatSpamThresholdSeconds() { return 0; }
        @Override public boolean useNativeTransport() { return false; }
        @Override public boolean isPublished() { return false; }
        @Override public boolean shouldInformAdmins() { return false; }
        @Override public boolean isSingleplayerOwner(NameAndId nameAndId) { return false; }
        @Override public int getMaxPlayers() { return 1; }
    }
}
