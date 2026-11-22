package dev.sunriselab.parity;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import net.minecraft.core.Holder;
import net.minecraft.core.RegistryAccess;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.waypoints.ServerWaypointManager;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntitySpawnRequest;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.ai.navigation.PathNavigation;
import net.minecraft.world.entity.animal.pig.Pig;
import net.minecraft.world.level.gameevent.GameEvent;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.VoxelShape;

/** Runs LivingEntity.tickDeath and observes real stopRiding calls on a nested passenger tree. */
public final class VanillaVehicleRemovalOracle {
    /** No chunks or packet transport: the removal and riding logic still run on a server level. */
    static final class Level extends ServerLevel {
        int next;
        ServerWaypointManager waypoints;

        Level() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }

        @Override public int getNextEntityId() { return ++next; }
        @Override public RegistryAccess registryAccess() { return ParityData.access; }
        @Override public ServerWaypointManager getWaypointManager() { return waypoints; }
        @Override public void broadcastEntityEvent(Entity entity, byte event) {}
        @Override public void gameEvent(Holder<GameEvent> event, Vec3 position, GameEvent.Context context) {}

        @Override
        public Optional<Vec3> findFreePosition(Entity subject, VoxelShape space, Vec3 center,
                                               double width, double height, double depth) {
            return Optional.empty();
        }
    }

    static final class Rider extends Pig {
        final int label;
        final List<String> trace;

        Rider(Level level, int label, List<String> trace) {
            super(EntityTypes.PIG, level);
            this.label = label;
            this.trace = trace;
            setPos(4.5, 64.0, 4.5);
        }

        // No AI runs in this fixture; navigation normally needs a server's debug service.
        @Override protected PathNavigation createNavigation(net.minecraft.world.level.Level level) { return null; }
        @Override protected void registerGoals() {}

        @Override
        public void stopRiding() {
            if (getVehicle() != null) {
                trace.add(label + ":removed=" + (isRemoved() || getVehicle().isRemoved()));
            }
            super.stopRiding();
        }

        void finishDeathTick() {
            setHealth(0);
            deathTime = 19;
            tickDeath();
        }
    }

    private static void mount(Entity rider, Entity vehicle) {
        if (!rider.startRiding(vehicle, true, false)) {
            throw new AssertionError("fixture's forced boarding was rejected");
        }
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        VanillaVehicleContainerOracle.reflect();
        Level level = (Level) VanillaVehicleContainerOracle.allocate.invoke(
            VanillaVehicleContainerOracle.unsafe, Level.class);
        level.waypoints = new ServerWaypointManager();
        var trace = new ArrayList<String>();
        var boat = EntityTypes.OAK_BOAT.create(level, new EntitySpawnRequest(EntitySpawnReason.LOAD, true));
        var parent = new Rider(level, 71, trace);
        var firstRow = new Rider(level, 72, trace);
        var secondRow = new Rider(level, 73, trace);
        var child = new Rider(level, 75, trace);
        mount(parent, boat);
        mount(secondRow, parent);
        mount(firstRow, parent);
        mount(child, firstRow);
        parent.finishDeathTick();
        // The next actor's observation, after the dying actor returns.
        trace.add("later:riding=" + firstRow.isPassenger());
        if (!parent.isRemoved() || secondRow.isPassenger() || child.getVehicle() != firstRow) {
            throw new AssertionError("unexpected final passenger tree");
        }
        Files.write(Path.of(args[0]), trace);
    }
}
