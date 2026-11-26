package dev.sunriselab.parity;

import com.google.common.collect.ImmutableList;
import com.google.gson.*;
import com.mojang.authlib.GameProfile;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.device.GpuDevice;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.function.Predicate;
import net.minecraft.client.*;
import net.minecraft.client.multiplayer.ClientChunkCache;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.client.player.AbstractClientPlayer;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.player.RemotePlayer;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.PacketProcessor;
import net.minecraft.network.protocol.game.ClientboundAddEntityPacket;
import net.minecraft.network.syncher.EntityDataAccessor;
import net.minecraft.network.syncher.SynchedEntityData;
import net.minecraft.resources.Identifier;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.TickRateManager;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.attributes.AttributeMap;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.boss.enderdragon.EnderDragon;
import net.minecraft.world.entity.boss.enderdragon.EnderDragonPart;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.Biomes;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.level.dimension.BuiltinDimensionTypes;
import net.minecraft.world.level.dimension.DimensionType;
import net.minecraft.world.level.entity.EntityTickList;
import net.minecraft.world.level.entity.LevelCallback;
import net.minecraft.world.level.entity.TransientEntitySectionManager;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.EntityHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;

// Vanilla 26.3 client oracle for crosshair targeting (#32 TNT/falling blocks, #34 root-vehicle exclusion).
public final class VanillaCrosshairPickOracle {
    static Object unsafe;
    static Method allocate;
    static Minecraft minecraft;
    static Options options;
    static DimensionType overworldDimType;

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static Field field(Class<?> owner, String name) throws Exception {
        for (Class<?> c = owner; c != null; c = c.getSuperclass()) {
            try {
                Field f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException ignored) {
            }
        }
        throw new NoSuchFieldException(owner.getName() + "." + name);
    }

    static Method method(Class<?> owner, String name, Class<?>... params) throws Exception {
        for (Class<?> c = owner; c != null; c = c.getSuperclass()) {
            try {
                Method m = c.getDeclaredMethod(name, params);
                m.setAccessible(true);
                return m;
            } catch (NoSuchMethodException ignored) {
            }
        }
        throw new NoSuchMethodException(owner.getName() + "." + name);
    }

    static String fb(float v) {
        return String.format("%08x", Float.isNaN(v) ? 0x7fc00000 : Float.floatToRawIntBits(v));
    }

    static String db(double v) {
        return String.format("%016x", Double.isNaN(v) ? 0x7ff8000000000000L : Double.doubleToRawLongBits(v));
    }

    static JsonArray vec3Bits(Vec3 v) {
        var a = new JsonArray();
        a.add(db(v.x));
        a.add(db(v.y));
        a.add(db(v.z));
        return a;
    }

    static JsonArray aabbBits(AABB b) {
        var a = new JsonArray();
        a.add(db(b.minX));
        a.add(db(b.minY));
        a.add(db(b.minZ));
        a.add(db(b.maxX));
        a.add(db(b.maxY));
        a.add(db(b.maxZ));
        return a;
    }

    static final class StubChunkCache extends ClientChunkCache {
        StubChunkCache() {
            super(null, 0);
        }

        @Override
        public boolean hasChunk(int x, int z) {
            return true;
        }
    }

    static final class StubClientLevel extends ClientLevel {
        Map<Integer, Entity> entities = new LinkedHashMap<>();
        Map<BlockPos, BlockState> blocks = new LinkedHashMap<>();
        TickRateManager tickRateManager = new TickRateManager();
        WorldBorder worldBorder = new WorldBorder();
        DimensionType dimType;
        StubChunkCache chunkCache;
        Holder<Biome> defaultBiome;
        net.minecraft.world.attribute.EnvironmentAttributeSystem envAttrs;
        TransientEntitySectionManager<Entity> storage;

        StubClientLevel() {
            super(null, null, null, null, 0, 0, null, false, 0L, 0);
        }

        void addTracked(Entity e) {
            entities.put(e.getId(), e);
            storage.addEntity(e);
        }

        @Override public boolean isClientSide() { return true; }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return net.minecraft.world.flag.FeatureFlags.VANILLA_SET; }
        @Override public RegistryAccess registryAccess() { return ParityData.access; }
        @Override public DimensionType dimensionType() { return dimType; }
        @Override public TickRateManager tickRateManager() { return tickRateManager; }
        @Override public WorldBorder getWorldBorder() { return worldBorder; }
        @Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes() { return envAttrs; }
        @Override public Entity getEntity(int id) { return storage.getEntityGetter().get(id); }
        @Override public Iterable<Entity> entitiesForRendering() { return storage.getEntityGetter().getAll(); }
        @Override public boolean isTickingEntity(Entity e) { return entities.containsKey(e.getId()); }
        @Override public BlockState getBlockState(BlockPos p) { return blocks.getOrDefault(p, Blocks.AIR.defaultBlockState()); }
        @Override public FluidState getFluidState(BlockPos p) { return Fluids.EMPTY.defaultFluidState(); }
        @Override public ClientChunkCache getChunkSource() { return chunkCache; }
        @Override public Holder<Biome> getBiome(BlockPos pos) { return defaultBiome; }
    }

    static final class StubSoundManager extends net.minecraft.client.sounds.SoundManager {
        StubSoundManager() {
            super(null);
        }

        @Override
        public net.minecraft.client.sounds.SoundEngine.PlayResult play(net.minecraft.client.resources.sounds.SoundInstance sound) {
            return net.minecraft.client.sounds.SoundEngine.PlayResult.NOT_STARTED;
        }
    }

    static final class StubLocalPlayer extends LocalPlayer {
        boolean spectatorFlag;
        Pose pose = Pose.STANDING;
        ItemStack mainHand = ItemStack.EMPTY;

        StubLocalPlayer() {
            super(null, null, null, null, null, net.minecraft.world.entity.player.Input.EMPTY, false, null, null);
        }

        @Override public boolean isSpectator() { return spectatorFlag; }
        @Override public boolean isUsingItem() { return false; }
        @Override public boolean isShiftKeyDown() { return false; }
        @Override public boolean isDescending() { return false; }
        @Override public ItemStack getMainHandItem() { return mainHand == null ? ItemStack.EMPTY : mainHand; }
        @Override public ItemStack getItemBySlot(EquipmentSlot slot) { return slot == EquipmentSlot.MAINHAND ? getMainHandItem() : ItemStack.EMPTY; }
        @Override public Vec3 getKnownMovement() { return Vec3.ZERO; }
        @Override public Pose getPose() { return pose; }
        @Override public void setPose(Pose p) { this.pose = p; }
    }

    static void installFakeGpuDevice() throws Exception {
        var deviceInfo = new com.mojang.renderpearl.api.device.DeviceInfo(
            "oracle", "oracle", "oracle", true, "oracle", 1.0F, null, null, Set.of(), null, null
        );
        Object device = Proxy.newProxyInstance(
            GpuDevice.class.getClassLoader(),
            new Class<?>[] { GpuDevice.class },
            (proxy, m, a) -> m.getName().equals("getDeviceInfo") ? deviceInfo : null
        );
        field(RenderSystem.class, "DEVICE").set(null, device);
    }

    @SuppressWarnings("unchecked")
    static EntityDataAccessor<Float> interactionWidthAccessor() throws Exception {
        return (EntityDataAccessor<Float>) field(Interaction.class, "DATA_WIDTH_ID").get(null);
    }

    @SuppressWarnings("unchecked")
    static EntityDataAccessor<Float> interactionHeightAccessor() throws Exception {
        return (EntityDataAccessor<Float>) field(Interaction.class, "DATA_HEIGHT_ID").get(null);
    }

    static StubClientLevel newLevel() throws Exception {
        var lvl = make(StubClientLevel.class);
        lvl.entities = new LinkedHashMap<>();
        lvl.blocks = new LinkedHashMap<>();
        lvl.tickRateManager = new TickRateManager();
        lvl.worldBorder = new WorldBorder();
        lvl.dimType = overworldDimType;
        lvl.envAttrs = net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().build();
        lvl.defaultBiome = ParityData.access.lookupOrThrow(Registries.BIOME).getOrThrow(Biomes.PLAINS);
        lvl.chunkCache = make(StubChunkCache.class);
        field(ClientLevel.class, "chunkSource").set(lvl, lvl.chunkCache);
        var levelData = new ClientLevel.ClientLevelData(net.minecraft.world.Difficulty.NORMAL, false, false);
        field(ClientLevel.class, "clientLevelData").set(lvl, levelData);
        field(Level.class, "levelData").set(lvl, levelData);
        field(Level.class, "dimension").set(lvl, Level.OVERWORLD);
        field(Level.class, "dimensionTypeRegistration").set(lvl, Holder.direct(overworldDimType));
        field(Level.class, "registryAccess").set(lvl, ParityData.access);
        field(Level.class, "isClientSide").setBoolean(lvl, true);
        field(Level.class, "random").set(lvl, net.minecraft.util.RandomSource.create(0L));
        field(ClientLevel.class, "minecraft").set(lvl, minecraft);
        var tickingEntities = new EntityTickList();
        var players = new ArrayList<AbstractClientPlayer>();
        var dragonParts = new ArrayList<EnderDragonPart>();
        field(ClientLevel.class, "tickingEntities").set(lvl, tickingEntities);
        field(ClientLevel.class, "players").set(lvl, players);
        field(ClientLevel.class, "dragonParts").set(lvl, dragonParts);
        var storage = new TransientEntitySectionManager<Entity>(Entity.class, new LevelCallback<Entity>() {
            @Override public void onCreated(Entity entity) {}
            @Override public void onDestroyed(Entity entity) {}
            @Override public void onTickingStart(Entity entity) { tickingEntities.add(entity); }
            @Override public void onTickingEnd(Entity entity) { tickingEntities.remove(entity); }
            @Override public void onTrackingStart(Entity entity) {
                if (entity instanceof AbstractClientPlayer player) {
                    players.add(player);
                } else if (entity instanceof EnderDragon dragon) {
                    dragonParts.addAll(Arrays.asList(dragon.getSubEntities()));
                }
            }
            @Override public void onTrackingEnd(Entity entity) {
                entity.unRide();
                if (entity instanceof AbstractClientPlayer player) {
                    players.remove(player);
                } else if (entity instanceof EnderDragon dragon) {
                    dragonParts.removeAll(Arrays.asList(dragon.getSubEntities()));
                }
            }
            @Override public void onSectionChange(Entity entity) {}
        });
        lvl.storage = storage;
        field(ClientLevel.class, "entityStorage").set(lvl, storage);
        return lvl;
    }

    static StubLocalPlayer newPlayer(StubClientLevel lvl, int id, double x, double y, double z, float yaw, float pitch, double blockReach, double entityReach) throws Exception {
        var p = make(StubLocalPlayer.class);
        p.pose = Pose.STANDING;
        p.mainHand = ItemStack.EMPTY;
        p.setId(id);
        field(Entity.class, "uuid").set(p, new UUID(0L, id));
        field(Entity.class, "type").set(p, EntityTypes.PLAYER);
        field(Entity.class, "level").set(p, lvl);
        field(LocalPlayer.class, "minecraft").set(p, minecraft);
        EntityDimensions dims = p.getDefaultDimensions(Pose.STANDING);
        field(Entity.class, "dimensions").set(p, dims);
        field(Entity.class, "eyeHeight").setFloat(p, dims.eyeHeight());
        field(Entity.class, "position").set(p, new Vec3(x, y, z));
        BlockPos bp = BlockPos.containing(x, y, z);
        field(Entity.class, "blockPosition").set(p, bp);
        field(Entity.class, "chunkPosition").set(p, net.minecraft.world.level.ChunkPos.containing(bp));
        field(Entity.class, "levelCallback").set(p, net.minecraft.world.level.entity.EntityInLevelCallback.NULL);
        p.xo = x; p.yo = y; p.zo = z;
        p.setYRot(yaw); p.yRotO = yaw; p.yHeadRot = yaw; p.yHeadRotO = yaw;
        p.setXRot(pitch); p.xRotO = pitch;
        p.setBoundingBox(dims.makeBoundingBox(x, y, z));
        field(Entity.class, "passengers").set(p, ImmutableList.of());
        field(LivingEntity.class, "activeEffects").set(p, new HashMap<>());
        var attrs = new AttributeMap(Player.createAttributes().build());
        attrs.getInstance(Attributes.BLOCK_INTERACTION_RANGE).setBaseValue(blockReach);
        attrs.getInstance(Attributes.ENTITY_INTERACTION_RANGE).setBaseValue(entityReach);
        field(LivingEntity.class, "attributes").set(p, attrs);
        var ab = new Abilities();
        ab.mayBuild = true;
        field(Player.class, "abilities").set(p, ab);
        var conn = make(ClientPacketListener.class);
        field(ClientPacketListener.class, "playerInfoMap").set(conn, new HashMap<>());
        field(LocalPlayer.class, "connection").set(p, conn);
        lvl.addTracked(p);
        minecraft.player = p;
        minecraft.level = lvl;
        return p;
    }

    static void setLook(Entity e, float yaw, float pitch) {
        e.setYRot(yaw);
        e.yRotO = yaw;
        e.setXRot(pitch);
        e.xRotO = pitch;
        if (e instanceof LivingEntity le) {
            le.yHeadRot = yaw;
            le.yHeadRotO = yaw;
        }
    }

    static void snapEntity(Entity e, double x, double y, double z) {
        e.setPos(x, y, z);
        e.xo = x;
        e.yo = y;
        e.zo = z;
    }

    static Entity spawnViaPacket(StubClientLevel lvl, String typeId, int id, double x, double y, double z, float yaw, float pitch, int data) throws Exception {
        if (typeId.equals("minecraft:player")) {
            var rp = new RemotePlayer(lvl, new GameProfile(new UUID(0L, id), "Remote" + id));
            var pkt = new ClientboundAddEntityPacket(id, new UUID(0L, id), x, y, z, pitch, yaw, EntityTypes.PLAYER, 0, Vec3.ZERO, yaw);
            rp.recreateFromPacket(pkt);
            rp.xo = x; rp.yo = y; rp.zo = z;
            rp.yRotO = yaw; rp.xRotO = pitch;
            lvl.addTracked(rp);
            return rp;
        }
        EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(typeId));
        Entity e = type.create(lvl, EntitySpawnReason.LOAD);
        if (e == null) throw new IllegalStateException("Could not create entity " + typeId);
        var pkt = new ClientboundAddEntityPacket(id, new UUID(0L, id), x, y, z, pitch, yaw, type, data, Vec3.ZERO, yaw);
        e.recreateFromPacket(pkt);
        if (e instanceof Interaction) {
            e.getEntityData().assignValues(List.of(
                SynchedEntityData.DataValue.create(interactionWidthAccessor(), 1.0F),
                SynchedEntityData.DataValue.create(interactionHeightAccessor(), 1.0F)
            ));
        }
        e.xo = x; e.yo = y; e.zo = z;
        e.yRotO = yaw; e.xRotO = pitch;
        lvl.addTracked(e);
        return e;
    }

    static void positionTree(Entity vehicle) throws Exception {
        Method posRider = method(Entity.class, "positionRider", Entity.class);
        for (Entity p : vehicle.getPassengers()) {
            posRider.invoke(vehicle, p);
            p.xo = p.getX(); p.yo = p.getY(); p.zo = p.getZ();
            positionTree(p);
        }
    }

    static float[] lookAt(Vec3 from, Vec3 target) {
        double dx = target.x - from.x;
        double dy = target.y - from.y;
        double dz = target.z - from.z;
        double horiz = Math.sqrt(dx * dx + dz * dz);
        float yaw = (float) (Math.toDegrees(Math.atan2(dz, dx)) - 90.0);
        float pitch = (float) (-Math.toDegrees(Math.atan2(dy, horiz)));
        return new float[] { yaw, pitch };
    }

    static final class Scene {
        final String name;
        final String category;
        final StubClientLevel lvl;
        final StubLocalPlayer player;
        final double blockReach;
        final double entityReach;
        final Map<Integer, String> kinds = new LinkedHashMap<>();
        final Map<Integer, Integer> spawnData = new LinkedHashMap<>();
        final Map<Integer, String> blockStateNames = new LinkedHashMap<>();
        final List<int[]> rideEdges = new ArrayList<>();
        final List<JsonObject> postMoves = new ArrayList<>();
        final Map<Integer, Integer> playerModes = new LinkedHashMap<>();
        final Map<Integer, Float> dragonHealth = new LinkedHashMap<>();
        String mainHandItem;
        float partialTick = 1.0F;
        Entity camera;

        Scene(String name, String category, double px, double py, double pz, float yaw, float pitch, double blockReach, double entityReach) throws Exception {
            this.name = name;
            this.category = category;
            this.blockReach = blockReach;
            this.entityReach = entityReach;
            this.lvl = newLevel();
            this.player = newPlayer(lvl, 1, px, py, pz, yaw, pitch, blockReach, entityReach);
            this.camera = this.player;
            this.kinds.put(1, "minecraft:player");
            this.spawnData.put(1, 0);
        }

        Entity add(String kind, int id, double x, double y, double z) throws Exception {
            return add(kind, id, x, y, z, 0.0F, 0.0F, 0, null);
        }

        Entity addFalling(int id, double x, double y, double z, Block block, String blockName) throws Exception {
            return add("minecraft:falling_block", id, x, y, z, 0.0F, 0.0F, Block.getId(block.defaultBlockState()), blockName);
        }

        Entity add(String kind, int id, double x, double y, double z, float yaw, float pitch, int data, String blockName) throws Exception {
            Entity e = spawnViaPacket(lvl, kind, id, x, y, z, yaw, pitch, data);
            kinds.put(id, kind);
            spawnData.put(id, data);
            if (blockName != null) blockStateNames.put(id, blockName);
            return e;
        }

        void block(int x, int y, int z, BlockState state) {
            lvl.blocks.put(new BlockPos(x, y, z), state);
        }

        void mount(Entity vehicle, Entity... passengers) throws Exception {
            int[] edge = new int[passengers.length + 1];
            edge[0] = vehicle.getId();
            for (int i = 0; i < passengers.length; i++) {
                boolean ok = passengers[i].startRiding(vehicle, true, false);
                if (!ok) {
                    throw new IllegalStateException(name + ": startRiding failed for " + passengers[i].getId() + " -> " + vehicle.getId());
                }
                edge[i + 1] = passengers[i].getId();
            }
            rideEdges.add(edge);
        }

        void settlePositions() throws Exception {
            for (Entity e : lvl.entities.values()) {
                if (!e.isPassenger()) {
                    positionTree(e);
                }
            }
        }

        @SuppressWarnings("unchecked")
        void setPlayerMode(RemotePlayer rp, GameType mode) throws Exception {
            var info = new PlayerInfo(rp.getGameProfile(), false);
            field(PlayerInfo.class, "gameMode").set(info, mode);
            var map = (Map<UUID, PlayerInfo>) field(ClientPacketListener.class, "playerInfoMap").get(player.connection);
            map.put(rp.getUUID(), info);
            playerModes.put(rp.getId(), mode.getId());
        }

        void moveTracked(Entity e, double x, double y, double z) {
            snapEntity(e, x, y, z);
            var m = new JsonObject();
            m.addProperty("id", e.getId());
            m.add("pos_bits", vec3Bits(e.position()));
            postMoves.add(m);
        }

        void displaceCamera(double oldX, double oldY, double oldZ, double newX, double newY, double newZ, float partial) {
            camera.setPos(newX, newY, newZ);
            camera.xo = oldX;
            camera.yo = oldY;
            camera.zo = oldZ;
            this.partialTick = partial;
        }

        void holdMainHand(String itemId, ItemStack stack) {
            player.mainHand = stack;
            this.mainHandItem = itemId;
        }

        void aimAtPoint(Vec3 target) {
            float[] rot = lookAt(camera.getEyePosition(partialTick), target);
            setLook(camera, rot[0], rot[1]);
        }

        JsonObject record() throws Exception {
            HitResult result = player.raycastHitResult(partialTick, camera);
            Vec3 eye = camera.getEyePosition(partialTick);
            Vec3 look = camera.getViewVector(partialTick);

            var row = new JsonObject();
            row.addProperty("name", name);
            row.addProperty("category", category);
            row.addProperty("block_reach_bits", db(blockReach));
            row.addProperty("entity_reach_bits", db(entityReach));
            row.addProperty("partial_tick_bits", fb(partialTick));
            if (mainHandItem != null) row.addProperty("main_hand_item", mainHandItem); else row.add("main_hand_item", JsonNull.INSTANCE);
            row.addProperty("player_id", player.getId());
            row.addProperty("camera_id", camera.getId());
            row.add("camera_pos_bits", vec3Bits(camera.position()));
            row.add("camera_prev_pos_bits", vec3Bits(new Vec3(camera.xo, camera.yo, camera.zo)));
            row.add("camera_eye_bits", vec3Bits(eye));
            row.addProperty("camera_eye_height_bits", fb(camera.getEyeHeight()));
            row.addProperty("camera_yRot_bits", fb(camera.getYRot()));
            row.addProperty("camera_xRot_bits", fb(camera.getXRot()));
            row.add("camera_look_bits", vec3Bits(look));
            row.addProperty("camera_root_vehicle_id", camera.getRootVehicle().getId());

            var blocksArr = new JsonArray();
            for (var entry : lvl.blocks.entrySet()) {
                var b = new JsonObject();
                var p = new JsonArray();
                p.add(entry.getKey().getX());
                p.add(entry.getKey().getY());
                p.add(entry.getKey().getZ());
                b.add("pos", p);
                b.addProperty("block", BuiltInRegistries.BLOCK.getKey(entry.getValue().getBlock()).toString());
                blocksArr.add(b);
            }
            row.add("blocks", blocksArr);

            var entsArr = new JsonArray();
            for (var entry : lvl.entities.entrySet()) {
                int id = entry.getKey();
                Entity e = entry.getValue();
                var eo = new JsonObject();
                eo.addProperty("id", id);
                eo.addProperty("kind", kinds.get(id));
                eo.addProperty("data", spawnData.getOrDefault(id, 0));
                if (blockStateNames.containsKey(id)) {
                    eo.addProperty("block_state", blockStateNames.get(id));
                } else {
                    eo.add("block_state", JsonNull.INSTANCE);
                }
                eo.add("pos_bits", vec3Bits(e.position()));
                eo.add("aabb_bits", aabbBits(e.getBoundingBox()));
                eo.addProperty("yRot_bits", fb(e.getYRot()));
                eo.addProperty("xRot_bits", fb(e.getXRot()));
                Entity veh = e.getVehicle();
                if (veh != null) eo.addProperty("vehicle_id", veh.getId()); else eo.add("vehicle_id", JsonNull.INSTANCE);
                eo.addProperty("root_vehicle_id", e.getRootVehicle().getId());
                eo.addProperty("is_pickable", e.isPickable());
                eo.addProperty("has_pick_result", e.getPickResult() != null);
                entsArr.add(eo);
            }
            row.add("entities", entsArr);

            var ridesArr = new JsonArray();
            for (int[] edge : rideEdges) {
                var ro = new JsonObject();
                ro.addProperty("vehicle", edge[0]);
                var ps = new JsonArray();
                for (int i = 1; i < edge.length; i++) ps.add(edge[i]);
                ro.add("passengers", ps);
                ridesArr.add(ro);
            }
            row.add("rides", ridesArr);

            var movesArr = new JsonArray();
            for (var m : postMoves) movesArr.add(m);
            row.add("moves", movesArr);

            var modesObj = new JsonObject();
            for (var entry : playerModes.entrySet()) modesObj.addProperty(String.valueOf(entry.getKey()), entry.getValue());
            row.add("player_modes", modesObj);

            var dragonHealthObj = new JsonObject();
            for (var entry : dragonHealth.entrySet()) dragonHealthObj.addProperty(String.valueOf(entry.getKey()), fb(entry.getValue()));
            row.add("dragon_health", dragonHealthObj);

            var dragonPartsArr = new JsonArray();
            for (Entity e : lvl.entities.values()) {
                if (e instanceof EnderDragon dragon) {
                    EnderDragonPart[] subs = dragon.getSubEntities();
                    for (int i = 0; i < subs.length; i++) {
                        var po = new JsonObject();
                        po.addProperty("index", i);
                        po.addProperty("id", subs[i].getId());
                        po.add("pos_bits", vec3Bits(subs[i].position()));
                        dragonPartsArr.add(po);
                    }
                }
            }
            row.add("dragon_parts", dragonPartsArr);

            var hitObj = new JsonObject();
            hitObj.addProperty("type", result.getType().name());
            hitObj.add("location_bits", vec3Bits(result.getLocation()));
            if (result instanceof EntityHitResult ehr) {
                hitObj.addProperty("entity_id", ehr.getEntity().getId());
                hitObj.add("block_pos", JsonNull.INSTANCE);
                hitObj.add("block_face", JsonNull.INSTANCE);
                hitObj.add("inside", JsonNull.INSTANCE);
            } else if (result instanceof BlockHitResult bhr) {
                hitObj.add("entity_id", JsonNull.INSTANCE);
                var bp = new JsonArray();
                bp.add(bhr.getBlockPos().getX());
                bp.add(bhr.getBlockPos().getY());
                bp.add(bhr.getBlockPos().getZ());
                hitObj.add("block_pos", bp);
                hitObj.addProperty("block_face", bhr.getDirection().name());
                hitObj.addProperty("inside", bhr.isInside());
            } else {
                hitObj.add("entity_id", JsonNull.INSTANCE);
                hitObj.add("block_pos", JsonNull.INSTANCE);
                hitObj.add("block_face", JsonNull.INSTANCE);
                hitObj.add("inside", JsonNull.INSTANCE);
            }
            row.add("hit", hitObj);
            return row;
        }
    }

    static JsonObject recordEntityTraits() throws Exception {
        var lvl = newLevel();
        var player = newPlayer(lvl, 1, 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
        Entity tnt = spawnViaPacket(lvl, "minecraft:tnt", 10, 0.5, 64.0, 2.5, 0.0F, 0.0F, 0);
        Entity fb = spawnViaPacket(lvl, "minecraft:falling_block", 11, 2.5, 64.0, 2.5, 0.0F, 0.0F, Block.getId(Blocks.SAND.defaultBlockState()));
        Entity bullet = spawnViaPacket(lvl, "minecraft:shulker_bullet", 12, 4.5, 64.0, 2.5, 0.0F, 0.0F, 0);
        Entity fireball = spawnViaPacket(lvl, "minecraft:fireball", 13, 6.5, 64.0, 2.5, 0.0F, 0.0F, 0);
        Entity arrow = spawnViaPacket(lvl, "minecraft:arrow", 14, 8.5, 64.0, 2.5, 0.0F, 0.0F, 0);
        var out = new JsonObject();
        for (var pair : List.of(
            Map.entry("tnt", tnt),
            Map.entry("falling_block", fb),
            Map.entry("shulker_bullet", bullet),
            Map.entry("fireball", fireball),
            Map.entry("arrow", arrow)
        )) {
            Entity e = pair.getValue();
            InteractionResult inter = e.interact(player, InteractionHand.MAIN_HAND, Vec3.ZERO);
            var obj = new JsonObject();
            obj.addProperty("is_pickable", e.isPickable());
            obj.addProperty("can_be_picked_from_inside", e.canBePickedFromInside());
            obj.addProperty("pick_radius_bits", fb(e.getPickRadius()));
            obj.addProperty("has_pick_result", e.getPickResult() != null);
            obj.addProperty("is_attackable", e.isAttackable());
            obj.addProperty("interact_result", inter.getClass().getSimpleName());
            obj.add("aabb_at_origin_bits", aabbBits(e.getDimensions(Pose.STANDING).makeBoundingBox(0.0, 0.0, 0.0)));
            out.add(pair.getKey(), obj);
        }
        return out;
    }

    static JsonArray recordTntAndFallingCases() throws Exception {
        var cases = new JsonArray();
        BlockState stone = Blocks.STONE.defaultBlockState();

        // 1. Primed TNT straight ahead occluding a stone block behind it.
        {
            var s = new Scene("tnt_straight_occludes_block", "tnt_and_falling", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.add("minecraft:tnt", 10, 0.5, 65.1, 2.5);
            s.block(0, 65, 4, stone);
            cases.add(s.record());
        }
        // 2. Primed TNT at an oblique yaw and mild downward pitch.
        {
            var s = new Scene("tnt_oblique_yaw_pitch", "tnt_and_falling", 0.5, 64.0, 0.5, -35.0F, 6.0F, 4.5, 3.0);
            s.add("minecraft:tnt", 10, 1.85, 64.9, 2.35);
            cases.add(s.record());
        }
        // 3. Primed TNT at a steep upward oblique angle.
        {
            var s = new Scene("tnt_steep_upward_oblique", "tnt_and_falling", 0.5, 64.0, 0.5, 42.5F, -32.0F, 4.5, 3.0);
            s.add("minecraft:tnt", 10, -0.75, 66.25, 1.85);
            cases.add(s.record());
        }
        // 4. Primed TNT looking down into its top face.
        {
            var s = new Scene("tnt_downward_top_face", "tnt_and_falling", 0.5, 64.0, 0.5, 18.0F, 42.0F, 4.5, 3.0);
            s.add("minecraft:tnt", 10, 0.0, 63.3, 2.0);
            cases.add(s.record());
        }
        // 5. Primed TNT right inside survival entity reach (3.0) with a block behind.
        {
            var s = new Scene("tnt_near_reach_edge_hit", "tnt_and_falling", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.add("minecraft:tnt", 10, 0.5, 65.1, 3.9);
            s.block(0, 65, 4, stone);
            cases.add(s.record());
        }
        // 6. Primed TNT past entity reach (3.0) but in front of stone within block reach (4.5): occludes block -> MISS.
        {
            var s = new Scene("tnt_past_entity_reach_occludes_block", "tnt_and_falling", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.add("minecraft:tnt", 10, 0.5, 65.1, 4.2);
            s.block(0, 65, 4, stone);
            cases.add(s.record());
        }
        // 7. Primed TNT behind a stone block: stone block wins.
        {
            var s = new Scene("tnt_behind_block", "tnt_and_falling", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.block(0, 65, 2, stone);
            s.add("minecraft:tnt", 10, 0.5, 65.1, 3.2);
            cases.add(s.record());
        }
        // 8. Primed TNT containing the camera eye (inside hit).
        {
            var s = new Scene("tnt_from_inside", "tnt_and_falling", 0.5, 64.0, 0.5, 23.0F, -11.0F, 4.5, 3.0);
            s.add("minecraft:tnt", 10, 0.5, 65.1, 0.5);
            s.block(0, 65, 3, stone);
            cases.add(s.record());
        }
        // 9. Primed TNT in front of a mob: TNT wins.
        {
            var s = new Scene("tnt_in_front_of_mob", "tnt_and_falling", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.add("minecraft:tnt", 10, 0.5, 65.1, 2.0);
            s.add("minecraft:creeper", 11, 0.5, 64.5, 3.2);
            cases.add(s.record());
        }
        // 10. Falling sand straight ahead occluding a stone block behind it.
        {
            var s = new Scene("falling_sand_straight_occludes_block", "tnt_and_falling", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.addFalling(20, 0.5, 65.1, 2.5, Blocks.SAND, "minecraft:sand");
            s.block(0, 65, 4, stone);
            cases.add(s.record());
        }
        // 11. Falling anvil at an oblique yaw and pitch.
        {
            var s = new Scene("falling_anvil_oblique_yaw_pitch", "tnt_and_falling", 0.5, 64.0, 0.5, -32.5F, 4.5F, 4.5, 3.0);
            s.addFalling(20, 1.75, 64.95, 2.4, Blocks.ANVIL, "minecraft:anvil");
            cases.add(s.record());
        }
        // 12. Falling gravel at a steep upward oblique angle.
        {
            var s = new Scene("falling_gravel_steep_upward_oblique", "tnt_and_falling", 0.5, 64.0, 0.5, 40.0F, -29.5F, 4.5, 3.0);
            s.addFalling(20, -0.7, 66.2, 1.9, Blocks.GRAVEL, "minecraft:gravel");
            cases.add(s.record());
        }
        // 13. Falling sand looking down into its top face.
        {
            var s = new Scene("falling_sand_downward_top_face", "tnt_and_falling", 0.5, 64.0, 0.5, -14.0F, 44.0F, 4.5, 3.0);
            s.addFalling(20, 0.9, 63.3, 2.0, Blocks.SAND, "minecraft:sand");
            cases.add(s.record());
        }
        // 14. Falling sand past entity reach (3.0) in front of stone within block reach (4.5): occludes block -> MISS.
        {
            var s = new Scene("falling_sand_past_entity_reach_occludes_block", "tnt_and_falling", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.addFalling(20, 0.5, 65.1, 4.2, Blocks.SAND, "minecraft:sand");
            s.block(0, 65, 4, stone);
            cases.add(s.record());
        }
        // 15. Falling sand behind a stone block: stone block wins.
        {
            var s = new Scene("falling_sand_behind_block", "tnt_and_falling", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.block(0, 65, 2, stone);
            s.addFalling(20, 0.5, 65.1, 3.2, Blocks.SAND, "minecraft:sand");
            cases.add(s.record());
        }
        // 16. Falling sand containing the camera eye (inside hit).
        {
            var s = new Scene("falling_sand_from_inside", "tnt_and_falling", 0.5, 64.0, 0.5, -17.0F, 9.0F, 4.5, 3.0);
            s.addFalling(20, 0.5, 65.1, 0.5, Blocks.SAND, "minecraft:sand");
            s.block(0, 65, 3, stone);
            cases.add(s.record());
        }
        // 17. Falling sand closer than primed TNT, both closer than a mob.
        {
            var s = new Scene("falling_sand_closer_than_tnt_and_mob", "tnt_and_falling", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.addFalling(20, 0.5, 65.1, 2.0, Blocks.SAND, "minecraft:sand");
            s.add("minecraft:tnt", 21, 0.5, 65.1, 2.6);
            s.add("minecraft:creeper", 22, 0.5, 64.5, 3.1);
            cases.add(s.record());
        }
        // 18. Extended creative reach (5.0 entity reach) hits TNT at distance 4.01.
        {
            var s = new Scene("tnt_extended_entity_reach_5", "tnt_and_falling", 0.5, 64.0, 0.5, 0.0F, 0.0F, 5.0, 5.0);
            s.add("minecraft:tnt", 10, 0.5, 65.1, 5.0);
            cases.add(s.record());
        }
        return cases;
    }

    static JsonArray recordNestedRideCases() throws Exception {
        var cases = new JsonArray();
        BlockState stone = Blocks.STONE.defaultBlockState();

        // A1: Player in a boat on a zombie, aiming down at the root zombie -> excluded, hits floor block beneath.
        {
            var s = new Scene("boat_on_zombie_aim_root_zombie", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity zombie = s.add("minecraft:zombie", 101, 0.5, 64.0, 0.5);
            Entity boat = s.add("minecraft:oak_boat", 102, 0.5, 64.0, 0.5);
            s.mount(zombie, boat);
            s.mount(boat, s.player);
            s.settlePositions();
            s.block(0, 63, 0, stone);
            s.aimAtPoint(zombie.getBoundingBox().getCenter());
            cases.add(s.record());
        }
        // A2: Player in a boat on a zombie with a villager co-passenger in the boat, aiming through villager at creeper.
        {
            var s = new Scene("boat_on_zombie_aim_copassenger_hits_creeper", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity zombie = s.add("minecraft:zombie", 101, 0.5, 64.0, 0.5);
            Entity boat = s.add("minecraft:oak_boat", 102, 0.5, 64.0, 0.5);
            Entity villager = s.add("minecraft:villager", 103, 0.5, 64.0, 0.5);
            s.mount(zombie, boat);
            s.mount(boat, s.player, villager);
            s.settlePositions();
            // Aim from player's eye directly through villager's center to a creeper behind.
            Vec3 eye = s.player.getEyePosition(1.0F);
            Vec3 dir = villager.getBoundingBox().getCenter().subtract(eye).normalize();
            Vec3 behind = eye.add(dir.scale(2.4));
            s.add("minecraft:creeper", 200, behind.x, behind.y - 0.85, behind.z);
            s.aimAtPoint(villager.getBoundingBox().getCenter());
            cases.add(s.record());
        }
        // A3: Player in a boat on an armor stand, with an interaction riding the armor stand; aim through interaction at TNT.
        {
            var s = new Scene("boat_on_armor_stand_aim_sibling_interaction_hits_tnt", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity stand = s.add("minecraft:armor_stand", 101, 0.5, 64.0, 0.5);
            Entity boat = s.add("minecraft:oak_boat", 102, 0.5, 64.0, 0.5);
            Entity inter = s.add("minecraft:interaction", 104, 0.5, 64.0, 0.5);
            s.mount(stand, boat, inter);
            s.mount(boat, s.player);
            s.settlePositions();
            // Move interaction slightly forward in +Z while keeping its vehicle link to stand.
            snapEntity(inter, 0.5, s.player.getEyePosition(1.0F).y - 0.5, 1.7);
            s.add("minecraft:tnt", 201, 0.5, s.player.getEyePosition(1.0F).y - 0.49, 2.8);
            setLook(s.player, 0.0F, 0.0F);
            cases.add(s.record());
        }
        // A4: Player in a boat on primed TNT, aiming at the root TNT -> excluded, hits block behind.
        {
            var s = new Scene("boat_on_tnt_aim_root_tnt_excluded", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity tnt = s.add("minecraft:tnt", 101, 0.5, 64.0, 0.5);
            Entity boat = s.add("minecraft:oak_boat", 102, 0.5, 64.0, 0.5);
            s.mount(tnt, boat);
            s.mount(boat, s.player);
            s.settlePositions();
            s.block(0, 63, 0, stone);
            s.aimAtPoint(tnt.getBoundingBox().getCenter());
            cases.add(s.record());
        }
        // B1: Player on a horse with a villager co-passenger; aim through villager at creeper behind.
        {
            var s = new Scene("horse_with_villager_copassenger_hits_creeper", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity horse = s.add("minecraft:horse", 110, 0.5, 64.0, 0.5);
            Entity villager = s.add("minecraft:villager", 111, 0.5, 64.0, 0.5);
            s.mount(horse, s.player, villager);
            s.settlePositions();
            snapEntity(villager, 0.5, s.player.getY(), 1.6);
            s.add("minecraft:creeper", 200, 0.5, s.player.getY(), 2.8);
            setLook(s.player, 0.0F, 0.0F);
            cases.add(s.record());
        }
        // B2: Player on a horse with an armor stand co-passenger; aim through armor stand at falling sand behind.
        {
            var s = new Scene("horse_with_armor_stand_copassenger_hits_falling", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity horse = s.add("minecraft:horse", 110, 0.5, 64.0, 0.5);
            Entity stand = s.add("minecraft:armor_stand", 112, 0.5, 64.0, 0.5);
            s.mount(horse, s.player, stand);
            s.settlePositions();
            snapEntity(stand, 0.5, s.player.getY(), 1.6);
            s.addFalling(202, 0.5, s.player.getEyePosition(1.0F).y - 0.49, 2.8, Blocks.SAND, "minecraft:sand");
            setLook(s.player, 0.0F, 0.0F);
            cases.add(s.record());
        }
        // B3: Player on a horse with a pig riding the player (pig -> player -> horse); aim at pig -> excluded, hits block.
        {
            var s = new Scene("horse_with_pig_riding_player_excluded", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity horse = s.add("minecraft:horse", 110, 0.5, 64.0, 0.5);
            Entity pig = s.add("minecraft:pig", 113, 0.5, 64.0, 0.5);
            s.mount(horse, s.player);
            s.mount(s.player, pig);
            s.settlePositions();
            s.block(0, 69, 0, stone);
            s.aimAtPoint(pig.getBoundingBox().getCenter());
            cases.add(s.record());
        }
        // B4: Player on a horse with a primed TNT co-passenger; aim through TNT at creeper behind.
        {
            var s = new Scene("horse_with_tnt_copassenger_hits_creeper", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity horse = s.add("minecraft:horse", 110, 0.5, 64.0, 0.5);
            Entity tnt = s.add("minecraft:tnt", 114, 0.5, 64.0, 0.5);
            s.mount(horse, s.player, tnt);
            s.settlePositions();
            snapEntity(tnt, 0.5, s.player.getEyePosition(1.0F).y - 0.49, 1.7);
            s.add("minecraft:creeper", 200, 0.5, s.player.getY(), 2.9);
            setLook(s.player, 0.0F, 0.0F);
            cases.add(s.record());
        }
        // C1: 3-cart minecart chain (player -> cart1 -> cart2 -> cart3), aiming at middle cart2 -> excluded, hits TNT behind.
        {
            var s = new Scene("minecart_chain_aim_middle_cart_excluded", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity cart3 = s.add("minecraft:minecart", 123, 0.5, 64.0, 0.5);
            Entity cart2 = s.add("minecraft:minecart", 122, 0.5, 64.0, 0.5);
            Entity cart1 = s.add("minecraft:minecart", 121, 0.5, 64.0, 0.5);
            s.mount(cart3, cart2);
            s.mount(cart2, cart1);
            s.mount(cart1, s.player);
            s.settlePositions();
            // Place cart2 in front along +Z while keeping the 3-level chain player -> cart1 -> cart2 -> cart3.
            snapEntity(cart2, 0.5, s.player.getEyePosition(1.0F).y - 0.35, 1.7);
            s.add("minecraft:tnt", 201, 0.5, s.player.getEyePosition(1.0F).y - 0.49, 2.8);
            setLook(s.player, 0.0F, 0.0F);
            cases.add(s.record());
        }
        // C2: 3-cart minecart chain, aiming down at root cart3 -> excluded, hits stone floor beneath.
        {
            var s = new Scene("minecart_chain_aim_root_cart_excluded", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity cart3 = s.add("minecraft:minecart", 123, 0.5, 64.0, 0.5);
            Entity cart2 = s.add("minecraft:minecart", 122, 0.5, 64.0, 0.5);
            Entity cart1 = s.add("minecraft:minecart", 121, 0.5, 64.0, 0.5);
            s.mount(cart3, cart2);
            s.mount(cart2, cart1);
            s.mount(cart1, s.player);
            s.settlePositions();
            s.block(0, 63, 0, stone);
            s.aimAtPoint(cart3.getBoundingBox().getCenter());
            cases.add(s.record());
        }
        // C3: Branching minecart tree: cart3 carries [cart2, villager], cart2 carries cart1, cart1 carries player.
        {
            var s = new Scene("minecart_chain_cousin_passenger_excluded", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity cart3 = s.add("minecraft:minecart", 123, 0.5, 64.0, 0.5);
            Entity cart2 = s.add("minecraft:minecart", 122, 0.5, 64.0, 0.5);
            Entity cart1 = s.add("minecraft:minecart", 121, 0.5, 64.0, 0.5);
            Entity cousin = s.add("minecraft:villager", 124, 0.5, 64.0, 0.5);
            s.mount(cart3, cart2, cousin);
            s.mount(cart2, cart1);
            s.mount(cart1, s.player);
            s.settlePositions();
            snapEntity(cousin, 0.5, s.player.getY(), 1.7);
            s.add("minecraft:creeper", 200, 0.5, s.player.getY(), 2.9);
            setLook(s.player, 0.0F, 0.0F);
            cases.add(s.record());
        }
        // C4: Minecart chain rooted on a falling block (player -> cart1 -> cart2 -> falling_block).
        {
            var s = new Scene("minecart_chain_on_falling_block_root_excluded", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity fb = s.addFalling(125, 0.5, 64.0, 0.5, Blocks.SAND, "minecraft:sand");
            Entity cart2 = s.add("minecraft:minecart", 122, 0.5, 64.0, 0.5);
            Entity cart1 = s.add("minecraft:minecart", 121, 0.5, 64.0, 0.5);
            s.mount(fb, cart2);
            s.mount(cart2, cart1);
            s.mount(cart1, s.player);
            s.settlePositions();
            s.block(0, 63, 0, stone);
            s.aimAtPoint(fb.getBoundingBox().getCenter());
            cases.add(s.record());
        }
        // D1: Camel with two riders [player (front), husk (back)]; player turns around and aims through husk at creeper.
        {
            var s = new Scene("camel_two_riders_front_aims_through_back_husk", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity camel = s.add("minecraft:camel", 130, 0.5, 64.0, 0.5);
            Entity husk = s.add("minecraft:husk", 131, 0.5, 64.0, 0.5);
            s.mount(camel, s.player, husk);
            s.settlePositions();
            Vec3 eye = s.player.getEyePosition(1.0F);
            Vec3 huskCenter = husk.getBoundingBox().getCenter();
            Vec3 dir = huskCenter.subtract(eye).normalize();
            Vec3 behind = eye.add(dir.scale(2.5));
            s.add("minecraft:creeper", 200, behind.x, behind.y - 0.85, behind.z);
            s.aimAtPoint(huskCenter);
            cases.add(s.record());
        }
        // D2: Camel with two riders [husk (front), player (back)]; player aims straight forward through husk at creeper.
        {
            var s = new Scene("camel_two_riders_back_aims_through_front_husk", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity camel = s.add("minecraft:camel", 130, 0.5, 64.0, 0.5);
            Entity husk = s.add("minecraft:husk", 131, 0.5, 64.0, 0.5);
            s.mount(camel, husk, s.player);
            s.settlePositions();
            Vec3 eye = s.player.getEyePosition(1.0F);
            Vec3 huskCenter = husk.getBoundingBox().getCenter();
            Vec3 dir = huskCenter.subtract(eye).normalize();
            Vec3 behind = eye.add(dir.scale(2.5));
            s.add("minecraft:creeper", 200, behind.x, behind.y - 0.85, behind.z);
            s.aimAtPoint(huskCenter);
            cases.add(s.record());
        }
        // D3: Camel with two player riders [remote_player (front), player (back)]; back rider aims through front player at TNT.
        {
            var s = new Scene("camel_two_players_back_aims_through_front_player", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity camel = s.add("minecraft:camel", 130, 0.5, 64.0, 0.5);
            Entity other = s.add("minecraft:player", 132, 0.5, 64.0, 0.5);
            s.mount(camel, other, s.player);
            s.settlePositions();
            Vec3 eye = s.player.getEyePosition(1.0F);
            Vec3 otherCenter = other.getBoundingBox().getCenter();
            Vec3 dir = otherCenter.subtract(eye).normalize();
            Vec3 behind = eye.add(dir.scale(2.5));
            s.add("minecraft:tnt", 201, behind.x, behind.y - 0.49, behind.z);
            s.aimAtPoint(otherCenter);
            cases.add(s.record());
        }
        // D4: Camel with two riders; aiming at a second unrelated camel's body DOES hit it.
        {
            var s = new Scene("camel_two_riders_unrelated_camel_hit", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity camel1 = s.add("minecraft:camel", 130, 0.5, 64.0, 0.5);
            Entity husk1 = s.add("minecraft:husk", 131, 0.5, 64.0, 0.5);
            s.mount(camel1, husk1, s.player);
            Entity camel2 = s.add("minecraft:camel", 140, 0.5, 64.0, 2.8);
            Entity husk2 = s.add("minecraft:husk", 141, 0.5, 64.0, 2.8);
            s.mount(camel2, husk2);
            s.settlePositions();
            s.aimAtPoint(husk2.getBoundingBox().getCenter());
            cases.add(s.record());
        }
        // D5: Camel with two riders; aiming at an unrelated horse's rider ahead DOES hit that rider.
        {
            var s = new Scene("camel_two_riders_unrelated_rider_hit", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity camel1 = s.add("minecraft:camel", 130, 0.5, 64.0, 0.5);
            Entity husk1 = s.add("minecraft:husk", 131, 0.5, 64.0, 0.5);
            s.mount(camel1, husk1, s.player);
            Entity horse2 = s.add("minecraft:horse", 140, 0.5, 64.0, 2.3);
            Entity vil2 = s.add("minecraft:villager", 141, 0.5, 64.0, 2.3);
            s.mount(horse2, vil2);
            s.settlePositions();
            s.aimAtPoint(vil2.getEyePosition(1.0F));
            cases.add(s.record());
        }
        // E1: Spectated camera entity riding a camel excludes the camel and its co-passenger.
        {
            var s = new Scene("spectated_camel_rider_excludes_copassenger", "nested_rides", 50.5, 64.0, 50.5, 0.0F, 0.0F, 4.5, 3.0);
            s.player.spectatorFlag = true;
            Entity camel = s.add("minecraft:camel", 130, 0.5, 64.0, 0.5);
            Entity husk = s.add("minecraft:husk", 131, 0.5, 64.0, 0.5);
            Entity villager = s.add("minecraft:villager", 133, 0.5, 64.0, 0.5);
            s.mount(camel, villager, husk);
            s.settlePositions();
            s.camera = husk;
            Vec3 eye = husk.getEyePosition(1.0F);
            Vec3 vilCenter = villager.getBoundingBox().getCenter();
            Vec3 dir = vilCenter.subtract(eye).normalize();
            Vec3 behind = eye.add(dir.scale(2.5));
            s.add("minecraft:creeper", 200, behind.x, behind.y - 0.85, behind.z);
            s.aimAtPoint(vilCenter);
            cases.add(s.record());
        }
        // E2: Unmounted player carrying a pig passenger (pig -> player); aiming up at pig excludes it and hits ceiling stone.
        {
            var s = new Scene("unmounted_player_carrying_pig_excluded", "nested_rides", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity pig = s.add("minecraft:pig", 150, 0.5, 64.0, 0.5);
            s.mount(s.player, pig);
            s.settlePositions();
            s.block(0, 68, 0, stone);
            s.aimAtPoint(pig.getBoundingBox().getCenter());
            cases.add(s.record());
        }

        return cases;
    }

    static JsonArray recordReviewParityCases() throws Exception {
        var cases = new JsonArray();
        BlockState stone = Blocks.STONE.defaultBlockState();
        BlockState chest = Blocks.CHEST.defaultBlockState();

        // R1 (Finding 1): Negative dragon id (-10) keeps wrapping arithmetic without underflow/u32::try_from failure.
        {
            var s = new Scene("dragon_negative_id_parts", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            EnderDragon dragon = (EnderDragon) s.add("minecraft:ender_dragon", -10, 0.5, 64.0, 2.5);
            EnderDragonPart body = dragon.getSubEntities()[2];
            body.setPos(0.5, 64.0, 2.5);
            s.aimAtPoint(body.getBoundingBox().getCenter());
            cases.add(s.record());
        }
        // R2 (Finding 1): Dragon id = Integer.MAX_VALUE - 2 wraps part ids across i32::MAX into i32::MIN.
        {
            var s = new Scene("dragon_wrapped_part_id_across_int_max", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            EnderDragon dragon = (EnderDragon) s.add("minecraft:ender_dragon", Integer.MAX_VALUE - 2, 0.5, 64.0, 2.5);
            EnderDragonPart part3 = dragon.getSubEntities()[3];
            part3.setPos(0.5, 64.0, 2.5);
            s.aimAtPoint(part3.getBoundingBox().getCenter());
            cases.add(s.record());
        }
        // R3 (Finding 6): Dying tracked dragon (health == 0) keeps its parts pickable.
        {
            var s = new Scene("dragon_dying_parts_still_pickable", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            EnderDragon dragon = (EnderDragon) s.add("minecraft:ender_dragon", 300, 0.5, 64.0, 2.5);
            dragon.setHealth(0.0F);
            s.dragonHealth.put(300, 0.0F);
            EnderDragonPart head = dragon.getSubEntities()[0];
            head.setPos(0.5, 65.0, 2.5);
            s.aimAtPoint(head.getBoundingBox().getCenter());
            cases.add(s.record());
        }
        // R4 (Finding 6): Spectating a dragon excludes all of its own sub-entity parts.
        {
            var s = new Scene("spectating_dragon_excludes_own_parts", "review_parity", 50.5, 64.0, 50.5, 0.0F, 0.0F, 4.5, 3.0);
            s.player.spectatorFlag = true;
            EnderDragon dragon = (EnderDragon) s.add("minecraft:ender_dragon", 300, 0.5, 64.0, 0.5);
            EnderDragonPart head = dragon.getSubEntities()[0];
            head.setPos(0.5, 64.0 + dragon.getEyeHeight() - 0.5, 1.5);
            s.add("minecraft:pig", 320, 0.5, 64.0 + dragon.getEyeHeight() - 0.45, 2.6);
            s.camera = dragon;
            setLook(dragon, 0.0F, 0.0F);
            cases.add(s.record());
        }
        // R5 (Finding 3): Spectator remote player is skipped by CAN_BE_PICKED so pig behind wins.
        {
            var s = new Scene("spectator_remote_player_excluded", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            RemotePlayer spec = (RemotePlayer) s.add("minecraft:player", 330, 0.5, 64.0, 1.8);
            s.setPlayerMode(spec, GameType.SPECTATOR);
            s.add("minecraft:pig", 331, 0.5, 65.0, 2.8);
            cases.add(s.record());
        }
        // R6 (Finding 4): Shulker bullet inherits Projectile.getPickRadius() == 1.0F.
        {
            var s = new Scene("shulker_bullet_pick_radius_one", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            // Bullet box is 0.3125 wide (half=0.15625); at x=1.35 it is 0.85 off-axis (missed at 0.0 radius, hit at 1.0).
            s.add("minecraft:shulker_bullet", 340, 1.35, 65.5, 2.5);
            cases.add(s.record());
        }
        // R7 (Finding 7): LeashFenceKnotEntity is an ordinary pickable candidate and occludes a pig behind it.
        {
            var s = new Scene("leash_knot_occludes_mob", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.add("minecraft:leash_knot", 350, 0.0, 65.0, 2.0);
            s.add("minecraft:pig", 351, 0.5, 65.0, 2.9);
            cases.add(s.record());
        }
        // R8 (Finding 5): Equal-distance tie across X sections: negative-X section wins over earlier-spawned positive-X.
        {
            var s = new Scene("section_order_x_tie_negative_x_wins", "review_parity", 0.0, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            // Both pigs have width 0.9 (half=0.45): x=0.35 (section 0) and x=-0.35 (section -1) both cross x=0 at exact z=2.05.
            s.add("minecraft:pig", 360, 0.35, 65.0, 2.5);
            s.add("minecraft:pig", 361, -0.35, 65.0, 2.5);
            cases.add(s.record());
        }
        // R9 (Finding 5): Moving an entity out of a section and back places it at the end of that section.
        {
            var s = new Scene("section_move_reorders_same_section_tie", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            Entity first = s.add("minecraft:pig", 370, 0.5, 65.0, 2.5);
            s.add("minecraft:pig", 371, 0.5, 65.0, 2.5);
            s.moveTracked(first, 20.5, 65.0, 2.5);
            s.moveTracked(first, 0.5, 65.0, 2.5);
            cases.add(s.record());
        }
        // R10 (Finding 5): Displaced camera at partialTick=0.5 sweeps its CURRENT bounding box.
        {
            var s = new Scene("displaced_camera_partial_tick_half", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.displaceCamera(0.5, 64.0, 0.5, 2.5, 64.0, 0.5, 0.5F);
            // Eye at partial=0.5 is x=1.5; TNT at x=1.5, z=2.5 is inside the swept current box (x in [2.2..2.8] inflated by 1.0 -> [1.2..3.8]).
            s.add("minecraft:tnt", 380, 1.5, 65.1, 2.5);
            cases.add(s.record());
        }
        // R10b (Finding 5): Displaced camera at partialTick=0.0 sweeps CURRENT bounding box (at x=4.5), missing TNT ahead of old eye (at x=0.5).
        {
            var s = new Scene("displaced_camera_current_box_excludes_entity_ahead_of_old_eye", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.displaceCamera(0.5, 64.0, 0.5, 4.5, 64.0, 0.5, 0.0F);
            s.add("minecraft:tnt", 381, 0.5, 65.1, 2.5);
            cases.add(s.record());
        }
        // R11 (Finding 8): Active AttackRange item picks TNT in front of a pig over the full pickable population.
        {
            var s = new Scene("attack_range_picks_tnt_over_pig", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.holdMainHand("minecraft:wooden_spear", new ItemStack(Items.WOODEN_SPEAR));
            s.add("minecraft:tnt", 390, 0.5, 65.1, 3.5);
            s.add("minecraft:pig", 391, 0.5, 65.0, 4.2);
            cases.add(s.record());
        }
        // R12 (Finding 8): Active AttackRange hitting a chest past min_reach keeps the block and does not fall back to TNT before min_reach.
        {
            var s = new Scene("attack_range_block_keeps_chest_not_tnt_before_min_reach", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.holdMainHand("minecraft:wooden_spear", new ItemStack(Items.WOODEN_SPEAR));
            s.add("minecraft:tnt", 392, 0.5, 65.1, 1.8);
            s.block(0, 65, 3, chest);
            cases.add(s.record());
        }
        // R13 (Finding 9): Out-of-reach TNT (at 3.2) in front of chest (at 3.5) with reaches (3.0, 4.5) resolves to MISS.
        {
            var s = new Scene("out_of_reach_tnt_in_front_of_chest_is_miss", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.add("minecraft:tnt", 395, 0.5, 65.1, 4.19);
            s.block(0, 65, 4, chest);
            cases.add(s.record());
        }
        // R14 (Finding 9): Block at exact block_reach (4.5) fails strict < filterHitResult -> MISS.
        {
            var s = new Scene("block_at_exact_block_reach_is_miss", "review_parity", 0.5, 64.0, 0.5, 0.0F, 0.0F, 4.5, 3.0);
            s.block(0, 65, 5, stone);
            cases.add(s.record());
        }

        return cases;
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var u = Class.forName("sun.misc.Unsafe");
        unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);

        installFakeGpuDevice();

        minecraft = make(Minecraft.class);
        field(Minecraft.class, "fixerUpper").set(minecraft, net.minecraft.util.datafix.DataFixers.getDataFixer());
        field(Minecraft.class, "instance").set(null, minecraft);
        field(Minecraft.class, "packetProcessor").set(minecraft, new PacketProcessor(Thread.currentThread()));
        field(Minecraft.class, "soundManager").set(minecraft, make(StubSoundManager.class));
        options = new Options(minecraft, Files.createTempDirectory("crosshair_pick_opts").toFile());
        field(Minecraft.class, "options").set(minecraft, options);

        overworldDimType = ParityData.access
            .lookupOrThrow(Registries.DIMENSION_TYPE)
            .getValueOrThrow(BuiltinDimensionTypes.OVERWORLD);

        var root = new JsonObject();
        root.add("entity_traits", recordEntityTraits());
        root.add("tnt_and_falling", recordTntAndFallingCases());
        root.add("nested_rides", recordNestedRideCases());
        root.add("review_parity", recordReviewParityCases());

        var gson = new GsonBuilder().setPrettyPrinting().serializeNulls().create();
        Files.writeString(Path.of(args[0]), gson.toJson(root) + "\n", StandardCharsets.UTF_8);
        System.exit(0);
    }
}
