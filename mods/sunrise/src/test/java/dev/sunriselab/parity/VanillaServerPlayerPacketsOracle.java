package dev.sunriselab.parity;

import com.google.gson.Gson;
import com.mojang.authlib.GameProfile;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.UUID;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.PositionAndRotation;
import net.minecraft.gametest.framework.GameTestServer;
import net.minecraft.nbt.TagParser;
import net.minecraft.network.Connection;
import net.minecraft.network.chat.Component;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.game.*;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.level.ClientInformation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.server.network.ServerGamePacketListenerImpl;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.util.ProblemReporter;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.decoration.ItemFrame;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.entity.PersistentEntitySectionManager;
import net.minecraft.world.level.entity.Visibility;
import net.minecraft.world.level.storage.LevelStorageSource;
import net.minecraft.world.level.storage.TagValueInput;
import net.minecraft.world.level.storage.TagValueOutput;
import net.minecraft.world.phys.Vec3;

/** Calls the real 26.3 play listener with synthetic serverbound packets on its server thread. */
public final class VanillaServerPlayerPacketsOracle {
    private static final Map<String, Object> rows = new LinkedHashMap<>();
    private static final Vec3 START = new Vec3(0.5, 100, 0.5);
    private static int nextProfile;

    private static final class Listener extends ServerGamePacketListenerImpl {
        final ArrayList<ClientboundPlayerPositionPacket> positions = new ArrayList<>();
        final ArrayList<Packet<?>> turn = new ArrayList<>();
        boolean disconnected;

        Listener(GameTestServer server, ServerPlayer player) {
            super(server, new Connection(PacketFlow.SERVERBOUND), player,
                CommonListenerCookie.createInitial(player.getGameProfile(), false));
        }

        @Override
        public void send(Packet<?> packet) {
            if (packet instanceof ClientboundPlayerPositionPacket position) positions.add(position);
            if (packet instanceof ClientboundPlayerPositionPacket || packet instanceof ClientboundSetCameraPacket
                || packet instanceof ClientboundSetPassengersPacket || packet instanceof ClientboundRespawnPacket) {
                turn.add(packet);
            }
        }

        @Override
        public void disconnect(Component reason) {
            disconnected = true;
        }
    }

    private static Field field(Class<?> type, String name) throws Exception {
        Field field = type.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }

    private static Listener player(GameTestServer server, ServerLevel level, boolean loaded) {
        int n = ++nextProfile;
        ServerPlayer player = new ServerPlayer(server, level,
            new GameProfile(new UUID(0, n), "PacketPlayer" + n), ClientInformation.createDefault());
        Listener listener = new Listener(server, player);
        player.snapTo(START.x, START.y, START.z, 17, -12);
        player.setGameMode(GameType.SURVIVAL);
        if (loaded) new ServerboundPlayerLoadedPacket().handle(listener);
        return listener;
    }

    private static long[] pose(ServerPlayer player) {
        return new long[] {Double.doubleToRawLongBits(player.getX()), Double.doubleToRawLongBits(player.getY()),
            Double.doubleToRawLongBits(player.getZ()), Float.floatToRawIntBits(player.getYRot()),
            Float.floatToRawIntBits(player.getXRot())};
    }

    private static void credits(GameTestServer server) throws Exception {
        ServerLevel end = server.getLevel(Level.END);
        Listener listener = player(server, end, true);
        end.addNewPlayer(listener.player);
        listener.player.showEndCredits();
        Field idle = field(ServerPlayer.class, "lastActionTime");
        idle.setLong(listener.player, 0);
        new ServerboundMovePlayerPacket.PosRot(0.75, 100, 0.5, 60, 30, true, false).handle(listener);
        rows.put("141/credits_pose", pose(listener.player));
        rows.put("141/credits_idle_credit", listener.player.getLastActionTime() != 0);
        rows.put("141/credits_move_count", field(ServerGamePacketListenerImpl.class, "receivedMovePacketCount").getInt(listener));
        new ServerboundMovePlayerPacket.Pos(0.75, 100, 0.5, true, false).handle(listener);
        rows.put("141/repeated_position", listener.disconnected);
        listener.disconnected = false;
        new ServerboundMovePlayerPacket.Rot(Float.NaN, 0, true, false).handle(listener);
        rows.put("141/invalid_rotation", listener.disconnected);

        Listener ordinary = player(server, server.overworld(), true);
        server.overworld().addNewPlayer(ordinary.player);
        idle.setLong(ordinary.player, 0);
        new ServerboundMovePlayerPacket.Pos(0.75, 100, 0.5, false, false).handle(ordinary);
        rows.put("141/ordinary_idle_credit", ordinary.player.getLastActionTime() != 0);
    }

    private static void useItem(GameTestServer server) throws Exception {
        WorldDataConfiguration original = server.getWorldData().getDataConfiguration();
        for (String kind : new String[] {"empty", "disabled", "survival", "spectator"}) {
            Listener listener = player(server, server.overworld(), true);
            if (!kind.equals("empty")) listener.player.setItemInHand(InteractionHand.OFF_HAND, new ItemStack(Items.SNOWBALL, 3));
            if (kind.equals("spectator")) listener.player.setGameMode(GameType.SPECTATOR);
            if (kind.equals("disabled")) server.getWorldData().setDataConfiguration(
                new WorldDataConfiguration(original.dataPacks(), FeatureFlagSet.of()));
            field(ServerPlayer.class, "lastActionTime").setLong(listener.player, 0);
            new ServerboundUseItemPacket(InteractionHand.OFF_HAND, 11, 725, -485).handle(listener);
            rows.put("142/" + kind, new int[] {Float.floatToRawIntBits(listener.player.getYRot()),
                Float.floatToRawIntBits(listener.player.getXRot()), listener.player.getOffhandItem().getCount(),
                field(ServerGamePacketListenerImpl.class, "ackBlockChangesUpTo").getInt(listener),
                listener.player.getLastActionTime() != 0 ? 1 : 0});
            server.getWorldData().setDataConfiguration(original);
        }
    }

    private static ServerboundPlayerInputPacket input(boolean shift) {
        return new ServerboundPlayerInputPacket(new Input(false, false, false, false, false, shift, false));
    }

    @SuppressWarnings("unchecked")
    private static void trackOrigin(ServerLevel level) throws Exception {
        // This fixture has no chunk tick loop. Mark the origin accessible so the real
        // entity section manager admits the test entities into the level's lookup.
        var manager = (PersistentEntitySectionManager<Entity>) field(ServerLevel.class, "entityManager").get(level);
        manager.updateChunkStatus(new ChunkPos(0, 0), Visibility.TRACKED);
    }

    private static void interact(GameTestServer server) {
        ServerLevel level = server.overworld();
        Listener listener = player(server, level, true);
        for (boolean shift : new boolean[] {true, false}) {
            input(!shift).handle(listener);
            new ServerboundInteractPacket(-1, InteractionHand.MAIN_HAND, Vec3.ZERO, shift).handle(listener);
            rows.put("143/missing_" + shift, listener.player.isShiftKeyDown());
        }
        for (boolean shift : new boolean[] {true, false}) {
            Listener rider = player(server, level, true);
            var boat = EntityTypes.OAK_BOAT.create(level, EntitySpawnReason.COMMAND);
            boat.setPos(1.5, 100, 0.5);
            if (!level.addFreshEntity(boat) || level.getEntity(boat.getUUID()) != boat) {
                throw new AssertionError("boat absent from level lookup");
            }
            input(!shift).handle(rider);
            new ServerboundInteractPacket(boat.getId(), InteractionHand.MAIN_HAND, Vec3.ZERO, shift).handle(rider);
            input(!shift).handle(rider);
            rows.put("143/boat_" + shift, rider.player.getVehicle() == boat);
            if (!shift) rows.put("143/next_input", rider.player.isShiftKeyDown());
        }
    }

    private static void spectate(GameTestServer server) throws Exception {
        ServerLevel overworld = server.overworld(), nether = server.getLevel(Level.NETHER);
        Listener viewer = player(server, overworld, true);
        viewer.player.setGameMode(GameType.SPECTATOR);
        overworld.addNewPlayer(viewer.player);
        Listener target = player(server, overworld, true);
        target.player.snapTo(10.5, 104, 7.5, -35, 20);
        overworld.addNewPlayer(target.player);
        new ServerboundTeleportToEntityPacket(target.player.getUUID()).handle(viewer);
        rows.put("144/player", pose(viewer.player));
        viewer.positions.clear();
        new ServerboundTeleportToEntityPacket(new UUID(1, 1)).handle(viewer);
        rows.put("144/missing", viewer.positions.size());

        var boat = EntityTypes.OAK_BOAT.create(overworld, EntitySpawnReason.COMMAND);
        boat.setPos(2.5, 100, 3.5);
        if (!overworld.addFreshEntity(boat)) throw new AssertionError("missing camera boat");
        for (boolean active : new boolean[] {false, true}) {
            if (active) viewer.player.setCamera(boat);
            if (!viewer.player.startRiding(boat, true, false)) throw new AssertionError("missing mount");
            viewer.player.fallDistance = 7.25;
            viewer.player.setDeltaMovement(new Vec3(0.125, -0.25, 0.5));
            viewer.turn.clear();
            new ServerboundTeleportToEntityPacket(target.player.getUUID()).handle(viewer);
            var packets = new ArrayList<String>();
            for (Packet<?> packet : viewer.turn) {
                if (packet instanceof ClientboundSetPassengersPacket riders
                    && riders.getVehicle() == boat.getId() && riders.getPassengers().length == 0) {
                    packets.add("dismount");
                } else if (packet instanceof ClientboundPlayerPositionPacket) {
                    packets.add("position");
                } else if (packet instanceof ClientboundSetCameraPacket camera
                    && camera.getEntity(overworld) == viewer.player) {
                    packets.add("camera_self");
                }
            }
            Class<?> listenerType = ServerGamePacketListenerImpl.class;
            boolean baselines = field(listenerType, "firstGoodX").getDouble(viewer) == viewer.player.getX()
                && field(listenerType, "firstGoodY").getDouble(viewer) == viewer.player.getY()
                && field(listenerType, "firstGoodZ").getDouble(viewer) == viewer.player.getZ()
                && field(listenerType, "lastGoodX").getDouble(viewer) == viewer.player.getX()
                && field(listenerType, "lastGoodY").getDouble(viewer) == viewer.player.getY()
                && field(listenerType, "lastGoodZ").getDouble(viewer) == viewer.player.getZ();
            rows.put("144/same_level_" + active, new Object[] {packets, viewer.player.getVehicle() == null,
                viewer.player.getCamera() == viewer.player, Double.doubleToRawLongBits(viewer.player.fallDistance),
                vectorBits(viewer.player.getDeltaMovement()), baselines});
        }

        Map<ServerLevel, UUID> stands = new LinkedHashMap<>();
        for (ServerLevel level : new ServerLevel[] {overworld, nether}) {
            var stand = EntityTypes.ARMOR_STAND.create(level, EntitySpawnReason.COMMAND);
            stand.snapTo(level == overworld ? 6.25 : 8.25, 101, 2.75, 63, 0);
            if (!level.addFreshEntity(stand) || level.getEntity(stand.getUUID()) != stand) {
                throw new AssertionError("stand absent from level lookup");
            }
            stands.put(level, stand.getUUID());
            new ServerboundTeleportToEntityPacket(stand.getUUID()).handle(viewer);
            rows.put("144/stand_" + level.dimension().identifier().getPath(), pose(viewer.player));
            rows.put("144/level_" + level.dimension().identifier().getPath(), viewer.player.level() == level);
        }
        var arrow = EntityTypes.ARROW.create(nether, EntitySpawnReason.COMMAND);
        arrow.snapTo(5.125, 101.5, 3.25, -123.25f, -32.5f);
        spectateEntity(viewer, nether, arrow, "arrow");
        var frame = new ItemFrame(overworld, new BlockPos(7, 100, 3), Direction.UP);
        spectateEntity(viewer, overworld, frame, "upward_frame");

        for (String name : List.of("mixed_return", "mixed_stay")) {
            viewer.turn.clear();
            viewer.positions.clear();
            new ServerboundTeleportToEntityPacket(stands.get(nether)).handle(viewer);
            UUID next = name.equals("mixed_return") ? stands.get(overworld) : arrow.getUUID();
            new ServerboundTeleportToEntityPacket(next).handle(viewer);
            var transitions = viewer.turn.stream().filter(ClientboundRespawnPacket.class::isInstance)
                .map(ClientboundRespawnPacket.class::cast)
                .map(p -> p.commonPlayerSpawnInfo().dimension().identifier().toString())
                .toList();
            rows.put("144/" + name, new Object[] {transitions, viewer.positions.size(),
                viewer.player.level().dimension().identifier().toString(), pose(viewer.player)});
        }
    }

    private static void spectateEntity(Listener viewer, ServerLevel level, Entity entity, String name) {
        if (!level.addFreshEntity(entity) || level.getEntity(entity.getUUID()) != entity) {
            throw new AssertionError(name + " absent from level lookup");
        }
        new ServerboundTeleportToEntityPacket(entity.getUUID()).handle(viewer);
        rows.put("144/" + name, pose(viewer.player));
    }

    private static long[] vectorBits(Vec3 vector) {
        return new long[] {Double.doubleToRawLongBits(vector.x), Double.doubleToRawLongBits(vector.y),
            Double.doubleToRawLongBits(vector.z)};
    }

    private static ServerboundMoveVehiclePacket vehiclePacket() {
        return new ServerboundMoveVehiclePacket(PositionAndRotation.of(new Vec3(2, 101, 3), 72, 35), true);
    }

    private static Object[] correctionSample(Listener listener) throws Exception {
        var corrections = listener.positions.stream().map(p -> new Object[] {
            p.id(), Float.floatToRawIntBits(p.change().yRot()), Float.floatToRawIntBits(p.change().xRot()),
            vectorBits(p.change().deltaMovement()),
        }).toList();
        Class<?> type = ServerGamePacketListenerImpl.class;
        Integer pending = field(type, "awaitingPositionFromClient").get(listener) == null
            ? null : field(type, "awaitingTeleport").getInt(listener);
        return new Object[] {corrections, Float.floatToRawIntBits(listener.player.getYRot()),
            Float.floatToRawIntBits(listener.player.getXRot()), pending};
    }

    private static void resend(GameTestServer server) throws Exception {
        Field tickCount = field(ServerGamePacketListenerImpl.class, "tickCount");
        for (String route : new String[] {"vehicle", "player", "loading_vehicle", "loading_player"}) {
            Listener listener = player(server, server.overworld(), !route.startsWith("loading_"));
            tickCount.setInt(listener, 100);
            listener.teleport(START.x, START.y, START.z, 17, -12);
            for (int elapsed : new int[] {19, 20, 21, 22}) {
                listener.positions.clear();
                tickCount.setInt(listener, 100 + elapsed);
                if (route.endsWith("vehicle")) {
                    vehiclePacket().handle(listener);
                } else {
                    new ServerboundMovePlayerPacket.Rot(72, 35, true, false).handle(listener);
                }
                rows.put("145/" + route + "_" + elapsed, correctionSample(listener));
            }
        }
        for (String kind : List.of("vehicle_then_accept", "status", "position")) {
            Listener listener = player(server, server.overworld(), true);
            float yaw = kind.equals("vehicle_then_accept") ? 17 : 270;
            tickCount.setInt(listener, 100);
            listener.teleport(START.x, START.y, START.z, yaw, -12);
            listener.positions.clear();
            tickCount.setInt(listener, 121);
            switch (kind) {
                case "vehicle_then_accept" -> {
                    vehiclePacket().handle(listener);
                    new ServerboundAcceptTeleportationPacket(1, START.x, START.y, START.z, yaw, -12).handle(listener);
                }
                case "status" -> new ServerboundMovePlayerPacket.StatusOnly(true, false).handle(listener);
                case "position" -> new ServerboundMovePlayerPacket.Pos(START.x, START.y, START.z, true, false).handle(listener);
            }
            rows.put("145/" + kind, correctionSample(listener));
        }
    }

    private static void crystalRoundTrip(GameTestServer server) throws Exception {
        ServerLevel level = server.overworld();
        var crystal = EntityTypes.END_CRYSTAL.create(level, EntitySpawnReason.LOAD);
        var tag = TagParser.parseCompoundFully("""
            {id:"minecraft:end_crystal",UUID:[I;0,0,0,130],Pos:[18.5d,100d,0.5d],
             Rotation:[-132.25f,31.5f],Invulnerable:1b,beam_target:[I;0,128,0],Fire:120s}
            """);
        crystal.load(TagValueInput.create(ProblemReporter.DISCARDING, level.registryAccess(), tag));
        BlockPos beam = crystal.getBeamTarget();
        rows.put("146/loaded_crystal", new Object[] {
            new int[] {Float.floatToRawIntBits(crystal.getYRot()), Float.floatToRawIntBits(crystal.getXRot())},
            crystal.isPermanentlyInvulnerable(), new int[] {beam.getX(), beam.getY(), beam.getZ()},
            crystal.getRemainingFireTicks(),
        });
        // Clear the two fields changed by EnderDragonFight.resetSpikeCrystals,
        // and run the real fire-immune crystal's tick before saving.
        crystal.setPermanentlyInvulnerable(false);
        crystal.setBeamTarget(null);
        crystal.tick();
        var out = TagValueOutput.createWithContext(ProblemReporter.DISCARDING, level.registryAccess());
        crystal.saveWithoutId(out);
        var saved = out.buildResult();
        boolean cleared = !saved.contains("beam_target") && !saved.getBooleanOr("Invulnerable", true)
            && saved.getShortOr("Fire", (short)-1) == 0;
        var reloaded = EntityTypes.END_CRYSTAL.create(level, EntitySpawnReason.LOAD);
        reloaded.load(TagValueInput.create(ProblemReporter.DISCARDING, level.registryAccess(), saved));
        rows.put("146/reloaded_crystal", new Object[] {
            new int[] {Float.floatToRawIntBits(reloaded.getYRot()), Float.floatToRawIntBits(reloaded.getXRot())},
            reloaded.isPermanentlyInvulnerable(), reloaded.getBeamTarget(), reloaded.getRemainingFireTicks(), cleared,
        });
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        try (var access = LevelStorageSource.createDefault(Path.of(args[0])).createAccess("packet-oracle")) {
            var server = GameTestServer.create(Thread.currentThread(), access,
                ServerPacksSource.createPackRepository(access), Optional.empty(), false, 0);
            var init = GameTestServer.class.getDeclaredMethod("initServer");
            init.setAccessible(true);
            init.invoke(server);
            trackOrigin(server.overworld());
            trackOrigin(server.getLevel(Level.NETHER));
            credits(server);
            useItem(server);
            interact(server);
            spectate(server);
            resend(server);
            crystalRoundTrip(server);
            Files.writeString(Path.of(args[1]), new Gson().toJson(rows));
        }
        System.exit(0);
    }
}
