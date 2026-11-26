package dev.sunriselab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.vertex.PoseStack;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.*;
import net.minecraft.client.entity.ClientAvatarState;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.state.GameRenderState;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.core.*;
import net.minecraft.util.Mth;
import net.minecraft.world.attribute.EnvironmentAttributeProbe;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.attributes.*;
import net.minecraft.world.entity.player.*;
import net.minecraft.world.item.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.*;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
import org.joml.Matrix4f;

/**
 * Drives the real Camera (tick, tickFov, alignWithEntity with getMaxZoom over
 * Level.clip, calculateFov, calculateHudFov), AbstractClientPlayer
 * .getFieldOfViewModifier and updateBob, ClientAvatarState and
 * GameRenderer.bobHurt/bobView with scripted entity states, and
 * AbstractBoat.clampRotation for a player rider, and prints every
 * observable bit-exactly. Entities, the level and the renderer are
 * uninitialised shells (Unsafe) with controlled queries; block visual shapes
 * come from the case.
 */
public final class VanillaCameraOracle {
    static Object unsafe;
    static Method allocate;

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static Field field(Class<?> type, String name) throws Exception {
        Field f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static Method method(Class<?> type, String name, Class<?>... args) throws Exception {
        Method m = type.getDeclaredMethod(name, args);
        m.setAccessible(true);
        return m;
    }

    static float f(JsonElement v) {
        return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(), 16));
    }

    static double d(JsonElement v) {
        return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(), 16));
    }

    static String fb(float v) {
        return String.format("%08x", Float.isNaN(v) ? 0x7fc00000 : Float.floatToRawIntBits(v));
    }

    static String db(double v) {
        return String.format("%016x", Double.isNaN(v) ? 0x7ff8000000000000L : Double.doubleToRawLongBits(v));
    }

    static Vec3 vec(JsonElement v) {
        var a = v.getAsJsonArray();
        return new Vec3(d(a.get(0)), d(a.get(1)), d(a.get(2)));
    }

    // ---- level with controlled visual shapes ----------------------------------------------
    static final class State extends BlockState {
        VoxelShape visual;

        State() {
            super(null, null, null);
        }

        @Override public VoxelShape getVisualShape(BlockGetter w, BlockPos p, CollisionContext c) { return visual; }
        @Override public VoxelShape getInteractionShape(BlockGetter w, BlockPos p) { return Shapes.empty(); }
    }

    static final class Level extends net.minecraft.server.level.ServerLevel {
        Map<BlockPos, BlockState> cells = new HashMap<>();
        State air;

        Level() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }

        @Override public BlockState getBlockState(BlockPos p) { return cells.getOrDefault(p, air); }
        @Override public FluidState getFluidState(BlockPos p) { return Fluids.EMPTY.defaultFluidState(); }
    }

    static final class NoProbe extends EnvironmentAttributeProbe {
        @Override public void tick(net.minecraft.world.level.Level level, Vec3 position) { }
    }

    static final class StubCamera extends Camera {
        net.minecraft.world.level.material.FogType fluid = net.minecraft.world.level.material.FogType.NONE;

        @Override public net.minecraft.world.level.material.FogType getFluidInCamera() { return fluid; }
    }

    // ---- camera entities ----------------------------------------------------------------
    static final class StubPlayer extends LocalPlayer {
        AttributeMap attributes;
        boolean using, dead, sleeping, swimming, ground;
        ItemStack useItem = ItemStack.EMPTY;
        int useTicks;
        Direction bed;
        Vec3 motion = Vec3.ZERO;

        StubPlayer() {
            super(null, null, null, null, null, Input.EMPTY, false, null, null);
        }

        @Override public AttributeMap getAttributes() { return attributes; }
        @Override public boolean isUsingItem() { return using; }
        @Override public ItemStack getUseItem() { return useItem; }
        @Override public int getTicksUsingItem() { return useTicks; }
        @Override public boolean isDeadOrDying() { return dead; }
        @Override public boolean isSleeping() { return sleeping; }
        @Override public Direction getBedOrientation() { return bed; }
        @Override public boolean isDescending() { return false; }
        @Override public ItemStack getMainHandItem() { return ItemStack.EMPTY; }
        @Override public boolean isSwimming() { return swimming; }
        @Override public boolean onGround() { return ground; }
        @Override public Vec3 getDeltaMovement() { return motion; }
        @Override public boolean isSpectator() { return false; }
    }

    static final class StubCreeper extends net.minecraft.world.entity.monster.Creeper {
        boolean dead;

        StubCreeper() {
            super(null, null);
        }

        @Override public boolean isDeadOrDying() { return dead; }
        @Override public boolean isSleeping() { return false; }
        @Override public boolean isDescending() { return false; }
    }

    static final class StubMarker extends Marker {
        StubMarker() {
            super(null, null);
        }

        @Override public boolean isDescending() { return false; }
    }

    static Level stubLevel;
    static Minecraft shell;
    static Options options;
    static Field position, xRot, yRot, eyeHeight, vehicle, abilities;
    static Constructor<ItemStack> stack;

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        shell = make(Minecraft.class);
        field(Minecraft.class, "fixerUpper").set(shell, net.minecraft.util.datafix.DataFixers.getDataFixer());
        field(Minecraft.class, "instance").set(null, shell);
        options = new Options(shell, Files.createTempDirectory("opts").toFile());
        field(Minecraft.class, "options").set(shell, options);
        position = field(Entity.class, "position");
        xRot = field(Entity.class, "xRot");
        yRot = field(Entity.class, "yRot");
        eyeHeight = field(Entity.class, "eyeHeight");
        vehicle = field(Entity.class, "vehicle");
        abilities = field(Player.class, "abilities");
        stack = ItemStack.class.getDeclaredConstructor(Holder.class, int.class, net.minecraft.core.component.PatchedDataComponentMap.class);
        stack.setAccessible(true);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            int n = 0;
            for (var line : (Iterable<String>) lines::iterator) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                out.println("== " + n++);
                switch (c.get("t").getAsString()) {
                    case "fov" -> out.println(fb(player(c).getFieldOfViewModifier(c.get("fp").getAsBoolean(), f(c.get("scale")))));
                    case "camera" -> camera(c, out);
                    case "bob" -> bob(c, out);
                    case "avatar" -> avatar(c, out);
                    case "boat" -> boat(c, out);
                    default -> throw new IllegalArgumentException(line);
                }
            }
        }
        System.exit(0);
    }

    /** The real AbstractBoat.clampRotation on a boat shell and a player shell riding it. */
    static void boat(JsonObject c, PrintWriter out) throws Exception {
        var clamp = method(net.minecraft.world.entity.vehicle.boat.AbstractBoat.class, "clampRotation", Entity.class);
        for (var pair : c.getAsJsonArray("views")) {
            var views = pair.getAsJsonArray();
            var rider = make(LocalPlayer.class);
            var boat = make(net.minecraft.world.entity.vehicle.boat.Boat.class);
            yRot.setFloat(rider, f(views.get(0)));
            yRot.setFloat(boat, f(views.get(1)));
            float turn = (float) clamp.invoke(boat, rider);
            out.println(fb(rider.getYRot()) + " " + fb(turn));
        }
    }

    static AttributeMap attributes(double scale, double distance, double speed) {
        var map = new AttributeMap(Player.createAttributes().build());
        map.getInstance(Attributes.SCALE).setBaseValue(scale);
        map.getInstance(Attributes.CAMERA_DISTANCE).setBaseValue(distance);
        map.getInstance(Attributes.MOVEMENT_SPEED).setBaseValue(speed);
        return map;
    }

    /** A player shell from the "fov" fields of a case or an entity op. */
    static StubPlayer player(JsonObject c) throws Exception {
        StubPlayer p = make(StubPlayer.class);
        fill(p, c);
        return p;
    }

    static void fill(StubPlayer p, JsonObject c) throws Exception {
        var a = new Abilities();
        a.flying = c.get("flying").getAsBoolean();
        a.setWalkingSpeed(f(c.get("ws")));
        abilities.set(p, a);
        double scale = c.has("scale_attr") ? d(c.get("scale_attr")) : 1.0;
        double distance = c.has("dist") ? d(c.get("dist")) : 4.0;
        p.attributes = attributes(scale, distance, d(c.get("speed")));
        var use = c.get("use");
        p.using = !use.isJsonNull();
        if (p.using) {
            var u = use.getAsJsonArray();
            Item item = switch (u.get(0).getAsString()) {
                case "bow" -> Items.BOW;
                case "spyglass" -> Items.SPYGLASS;
                default -> Items.STICK;
            };
            p.useItem = stack.newInstance(Holder.direct(item), 1,
                new net.minecraft.core.component.PatchedDataComponentMap(net.minecraft.core.component.DataComponentMap.EMPTY));
            p.useTicks = u.get(1).getAsInt();
        }
    }

    static void camera(JsonObject c, PrintWriter out) throws Exception {
        var level = make(Level.class);
        level.cells = new HashMap<>();
        level.air = make(State.class);
        level.air.visual = Shapes.empty();
        stubLevel = level;
        if (c.has("bed")) {
            var b = c.getAsJsonArray("bed");
            level.cells.put(new BlockPos(b.get(0).getAsInt(), b.get(1).getAsInt(), b.get(2).getAsInt()),
                net.minecraft.world.level.block.Blocks.BED.red().defaultBlockState()
                    .setValue(net.minecraft.world.level.block.BedBlock.FACING, Direction.from2DDataValue(b.get(3).getAsInt())));
        }
        for (var e : c.getAsJsonObject("cells").entrySet()) {
            var at = e.getKey().split(" ");
            var s = make(State.class);
            VoxelShape shape = Shapes.empty();
            for (var b : e.getValue().getAsJsonArray()) {
                var a = b.getAsJsonArray();
                shape = Shapes.or(shape, Shapes.box(d(a.get(0)), d(a.get(1)), d(a.get(2)), d(a.get(3)), d(a.get(4)), d(a.get(5))));
            }
            s.visual = shape;
            level.cells.put(new BlockPos(Integer.parseInt(at[0]), Integer.parseInt(at[1]), Integer.parseInt(at[2])), s);
        }
        options.fov().set(c.get("fov").getAsInt());
        options.fovEffectScale().set(d(c.get("effect")));
        options.setCameraType(CameraType.values()[c.get("type").getAsInt()]);
        var camera = new StubCamera();
        field(Camera.class, "attributeProbe").set(camera, new NoProbe());
        field(Camera.class, "level").set(camera, level);
        GameRenderer renderer = make(GameRenderer.class);
        field(GameRenderer.class, "mainCamera").set(renderer, camera);
        field(Minecraft.class, "gameRenderer").set(shell, renderer);
        var align = method(Camera.class, "alignWithEntity", float.class);
        var fov = method(Camera.class, "calculateFov", float.class);
        var hud = method(Camera.class, "calculateHudFov", float.class);
        var panoramic = field(Camera.class, "isPanoramicMode");
        var eye = field(Camera.class, "eyeHeight");
        var eyeOld = field(Camera.class, "eyeHeightOld");
        var mod = field(Camera.class, "fovModifier");
        var modOld = field(Camera.class, "oldFovModifier");
        Entity entity = null;
        for (var e : c.getAsJsonArray("ops")) {
            var op = e.getAsJsonArray();
            switch (op.get(0).getAsString()) {
                case "entity" -> {
                    var s = op.get(1).getAsJsonObject();
                    entity = entity(s, entity);
                    camera.setEntity(entity);
                    out.println("entity");
                }
                case "type" -> {
                    options.setCameraType(CameraType.values()[op.get(1).getAsInt()]);
                    out.println("type");
                }
                case "tick" -> {
                    camera.tick();
                    out.println("tick " + fb(eye.getFloat(camera)) + " " + fb(eyeOld.getFloat(camera)) + " "
                        + fb(mod.getFloat(camera)) + " " + fb(modOld.getFloat(camera)));
                }
                case "frame" -> {
                    float pt = f(op.get(1));
                    camera.fluid = net.minecraft.world.level.material.FogType.valueOf(
                        new String[] {"NONE", "WATER", "LAVA", "POWDER_SNOW"}[op.get(2).getAsInt()]);
                    panoramic.setBoolean(camera, op.get(3).getAsBoolean());
                    align.invoke(camera, pt);
                    float fv = (float) fov.invoke(camera, pt);
                    float hv = (float) hud.invoke(camera, pt);
                    var p = camera.position();
                    var q = camera.rotation();
                    var fw = camera.forwardVector();
                    var up = camera.upVector();
                    var left = camera.leftVector();
                    out.println("frame " + db(p.x) + " " + db(p.y) + " " + db(p.z) + " " + fb(camera.xRot()) + " " + fb(camera.yRot())
                        + " " + fb(q.x) + " " + fb(q.y) + " " + fb(q.z) + " " + fb(q.w)
                        + " " + fb(fw.x()) + " " + fb(fw.y()) + " " + fb(fw.z())
                        + " " + fb(up.x()) + " " + fb(up.y()) + " " + fb(up.z())
                        + " " + fb(left.x()) + " " + fb(left.y()) + " " + fb(left.z())
                        + " " + camera.isDetached() + " " + fb(fv) + " " + fb(hv));
                    if (c.has("report_distance")) {
                        float cameraScale = 1.0F, cameraDistance = 4.0F;
                        if (entity instanceof LivingEntity living) {
                            cameraScale = living.getScale();
                            cameraDistance = (float) living.getAttributeValue(Attributes.CAMERA_DISTANCE);
                        }
                        float mountScale = cameraScale, mountDistance = cameraDistance;
                        if (entity.isPassenger() && entity.getVehicle() instanceof LivingEntity mount) {
                            mountScale = mount.getScale();
                            mountDistance = (float) mount.getAttributeValue(Attributes.CAMERA_DISTANCE);
                        }
                        out.println("dist " + fb(Math.max(cameraScale * cameraDistance, mountScale * mountDistance)));
                    }
                    // Camera.setupPerspective and GameRenderer.render3dHud's hudProjection:
                    // Projection.getMatrix's setPerspective (planes reversed, zero-to-one depth).
                    float w = f(op.get(4)), h = f(op.get(5)), far = f(op.get(6));
                    out.println("projection" + matrix(new Matrix4f().setPerspective(fv * (float) (Math.PI / 180.0), w / h, far, 0.05F, true))
                        + matrix(new Matrix4f().setPerspective(hv * (float) (Math.PI / 180.0), w / h, far, 0.05F, true)));
                }
                default -> throw new IllegalArgumentException(op.toString());
            }
        }
    }

    /** The vehicle of a "mount": [scale, distance] is a player shell; a third element names a real vehicle class. */
    static Entity vehicleOf(com.google.gson.JsonElement mount) throws Exception {
        if (mount.isJsonNull()) {
            return null;
        }
        var a = mount.getAsJsonArray();
        String kind = a.size() > 2 ? a.get(2).getAsString() : "player";
        if (kind.equals("boat")) {
            return make(net.minecraft.world.entity.vehicle.boat.Boat.class);
        }
        Entity living;
        AttributeSupplier.Builder supplier;
        switch (kind) {
            case "happy_ghast" -> {
                living = make(net.minecraft.world.entity.animal.happyghast.HappyGhast.class);
                supplier = net.minecraft.world.entity.animal.happyghast.HappyGhast.createAttributes();
            }
            case "ghast" -> {
                living = make(net.minecraft.world.entity.monster.Ghast.class);
                supplier = net.minecraft.world.entity.monster.Ghast.createAttributes();
            }
            case "zombie" -> {
                living = make(net.minecraft.world.entity.monster.zombie.Zombie.class);
                supplier = net.minecraft.world.entity.monster.zombie.Zombie.createAttributes();
            }
            case "armor_stand" -> {
                living = make(net.minecraft.world.entity.decoration.ArmorStand.class);
                supplier = net.minecraft.world.entity.decoration.ArmorStand.createAttributes();
            }
            case "player" -> {
                StubPlayer m = make(StubPlayer.class);
                m.attributes = attributes(d(a.get(0)), d(a.get(1)), 0.1);
                return m;
            }
            default -> throw new IllegalArgumentException(kind);
        }
        var map = new AttributeMap(supplier.build());
        map.getInstance(Attributes.SCALE).setBaseValue(d(a.get(0)));
        map.getInstance(Attributes.CAMERA_DISTANCE).setBaseValue(d(a.get(1)));
        field(LivingEntity.class, "attributes").set(living, map);
        return living;
    }

    /** Applies an entity op: a player (with fov fields) or a non-living marker. */
    static Entity entity(JsonObject s, Entity previous) throws Exception {
        Entity entity;
        String kind = s.get("kind").getAsString();
        if (kind.equals("villager") || kind.equals("remote_player")) {
            entity = realLiving(s, kind);
        } else if (kind.equals("player")) {
            StubPlayer p = previous instanceof StubPlayer old ? old : make(StubPlayer.class);
            fill(p, s);
            p.dead = !s.get("dead").isJsonNull();
            if (p.dead) {
                p.deathTime = s.get("dead").getAsInt();
            }
            p.sleeping = !s.get("sleeping").isJsonNull();
            p.bed = null;
            if (p.sleeping && !s.get("sleeping").getAsJsonArray().get(0).isJsonNull()) {
                p.bed = Direction.from2DDataValue(s.get("sleeping").getAsJsonArray().get(0).getAsInt());
            }
            vehicle.set(p, vehicleOf(s.get("mount")));
            entity = p;
        } else if (s.get("kind").getAsString().equals("creeper")) {
            StubCreeper c = previous instanceof StubCreeper old ? old : make(StubCreeper.class);
            c.dead = !s.get("dead").isJsonNull();
            if (c.dead) {
                c.deathTime = s.get("dead").getAsInt();
            }
            entity = c;
        } else {
            entity = previous instanceof StubMarker old ? old : make(StubMarker.class);
            vehicle.set(entity, s.has("mount") ? vehicleOf(s.get("mount")) : null);
        }
        position.set(entity, vec(s.get("pos")));
        if (entity instanceof LivingEntity) {
            ((LivingEntity) entity).yHeadRot = f(s.get("yr"));
            ((LivingEntity) entity).yHeadRotO = f(s.get("yro"));
        }
        var prev = vec(s.get("prev"));
        entity.xo = prev.x;
        entity.yo = prev.y;
        entity.zo = prev.z;
        xRot.setFloat(entity, f(s.get("xr")));
        yRot.setFloat(entity, f(s.get("yr")));
        entity.xRotO = f(s.get("xro"));
        entity.yRotO = f(s.get("yro"));
        eyeHeight.setFloat(entity, f(s.get("eye")));
        return entity;
    }

    /** A real Villager or RemotePlayer (no constructor run): its own synched data, attributes and level, asleep at "sleeping_pos". */
    @SuppressWarnings({"unchecked", "rawtypes"})
    static Entity realLiving(JsonObject s, String kind) throws Exception {
        boolean villager = kind.equals("villager");
        LivingEntity e = villager ? make(net.minecraft.world.entity.npc.villager.Villager.class)
            : make(net.minecraft.client.player.RemotePlayer.class);
        field(Entity.class, "level").set(e, stubLevel);
        var builder = new net.minecraft.network.syncher.SynchedEntityData.Builder(e);
        for (var pair : new Object[][] {{"DATA_SHARED_FLAGS_ID", (byte) 0}, {"DATA_AIR_SUPPLY_ID", 300}, {"DATA_CUSTOM_NAME_VISIBLE", false},
            {"DATA_CUSTOM_NAME", Optional.empty()}, {"DATA_SILENT", false}, {"DATA_NO_GRAVITY", false}, {"DATA_POSE", Pose.STANDING},
            {"DATA_TICKS_FROZEN", 0}}) {
            builder.define((net.minecraft.network.syncher.EntityDataAccessor) field(Entity.class, (String) pair[0]).get(null), pair[1]);
        }
        method(Entity.class, "defineSynchedData", net.minecraft.network.syncher.SynchedEntityData.Builder.class).invoke(e, builder);
        field(Entity.class, "entityData").set(e, builder.build());
        var map = new AttributeMap(villager ? net.minecraft.world.entity.npc.villager.Villager.createAttributes().build()
            : Player.createAttributes().build());
        map.getInstance(Attributes.SCALE).setBaseValue(d(s.get("scale_attr")));
        map.getInstance(Attributes.CAMERA_DISTANCE).setBaseValue(d(s.get("dist")));
        field(LivingEntity.class, "attributes").set(e, map);
        if (!villager) {
            var a = new Abilities();
            a.flying = s.get("flying").getAsBoolean();
            a.setWalkingSpeed(f(s.get("ws")));
            abilities.set(e, a);
            map.getInstance(Attributes.MOVEMENT_SPEED).setBaseValue(d(s.get("speed")));
        }
        var at = s.getAsJsonArray("sleeping_pos");
        var accessor = (net.minecraft.network.syncher.EntityDataAccessor) field(LivingEntity.class, "SLEEPING_POS_ID").get(null);
        e.getEntityData().set(accessor, Optional.of(new BlockPos(at.get(0).getAsInt(), at.get(1).getAsInt(), at.get(2).getAsInt())));
        return e;
    }

    static void bob(JsonObject c, PrintWriter out) throws Exception {
        GameRenderer renderer = make(GameRenderer.class);
        var state = new GameRenderState();
        field(GameRenderer.class, "gameRenderState").set(renderer, state);
        state.optionsRenderState.damageTiltStrength = d(c.get("tilt"));
        state.optionsRenderState.bobView = c.get("view").getAsBoolean();
        var camera = new CameraRenderState();
        var es = camera.entityRenderState;
        var living = c.get("living");
        es.isLiving = !living.isJsonNull();
        if (es.isLiving) {
            var l = living.getAsJsonObject();
            es.hurtTime = f(l.get("hurt"));
            es.hurtDuration = l.get("duration").getAsInt();
            es.hurtDir = f(l.get("dir"));
            es.isDeadOrDying = !l.get("death").isJsonNull();
            if (es.isDeadOrDying) {
                es.deathTime = f(l.get("death"));
            }
        }
        var player = c.get("player");
        es.isPlayer = !player.isJsonNull();
        if (es.isPlayer) {
            es.backwardsInterpolatedWalkDistance = f(player.getAsJsonArray().get(0));
            es.bob = f(player.getAsJsonArray().get(1));
        }
        var stack = new PoseStack();
        method(GameRenderer.class, "bobHurt", CameraRenderState.class, PoseStack.class).invoke(renderer, camera, stack);
        if (state.optionsRenderState.bobView) {
            method(GameRenderer.class, "bobView", CameraRenderState.class, PoseStack.class).invoke(renderer, camera, stack);
        }
        Matrix4f m = stack.last().pose();
        var sb = new StringBuilder("bob");
        for (int col = 0; col < 4; col++) {
            for (int row = 0; row < 4; row++) {
                sb.append(' ').append(fb(m.get(col, row)));
            }
        }
        sb.append(' ').append(m.properties());
        out.println(sb);
    }

    static String matrix(Matrix4f m) {
        var sb = new StringBuilder();
        for (int col = 0; col < 4; col++) {
            for (int row = 0; row < 4; row++) {
                sb.append(' ').append(fb(m.get(col, row)));
            }
        }
        return sb.toString();
    }

    static void avatar(JsonObject c, PrintWriter out) throws Exception {
        StubPlayer p = make(StubPlayer.class);
        var state = new ClientAvatarState();
        field(net.minecraft.client.player.AbstractClientPlayer.class, "clientAvatarState").set(p, state);
        var updateBob = method(net.minecraft.client.player.AbstractClientPlayer.class, "updateBob");
        for (var e : c.getAsJsonArray("ops")) {
            var op = e.getAsJsonArray();
            switch (op.get(0).getAsString()) {
                case "tick" -> state.tick(Vec3.ZERO, Vec3.ZERO);
                case "move" -> state.addWalkDistance(Mth.length(f(op.get(1)), f(op.get(2))) * 0.6F);
                case "bob" -> {
                    p.ground = op.get(1).getAsBoolean();
                    p.dead = op.get(2).getAsBoolean();
                    p.swimming = op.get(3).getAsBoolean();
                    p.motion = vec(op.get(4));
                    updateBob.invoke(p);
                }
                case "reset" -> state.resetBob();
                case "frame" -> {
                    float pt = f(op.get(1));
                    out.println("frame " + fb(state.getInterpolatedBob(pt)) + " " + fb(state.getBackwardsInterpolatedWalkDistance(pt)));
                    continue;
                }
                default -> throw new IllegalArgumentException(op.toString());
            }
            out.println(op.get(0).getAsString());
        }
    }
}
