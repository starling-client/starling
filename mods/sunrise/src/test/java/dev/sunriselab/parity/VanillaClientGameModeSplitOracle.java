package dev.sunriselab.parity;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.mojang.authlib.GameProfile;
import it.unimi.dsi.fastutil.longs.LongOpenHashSet;
import it.unimi.dsi.fastutil.objects.ObjectArrayList;
import java.io.Writer;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.EnumSet;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.UUID;
import net.minecraft.client.Camera;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.entity.ClientAvatarState;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.gui.Hud;
import net.minecraft.client.gui.screens.LevelLoadingScreen;
import net.minecraft.client.gui.screens.social.PlayerSocialManager;
import net.minecraft.client.multiplayer.ClientCommonPacketListenerImpl;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.client.player.AbstractClientPlayer;
import net.minecraft.client.player.FirstPersonHandsAndItems;
import net.minecraft.client.player.KeyboardInput;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.player.RemotePlayer;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.tutorial.Tutorial;
import net.minecraft.client.waypoints.ClientWaypointManager;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.network.PacketProcessor;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientboundGameEventPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerAbilitiesPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoRemovePacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket;
import net.minecraft.network.protocol.game.ServerboundPlayerAbilitiesPacket;
import net.minecraft.tags.FluidTags;
import net.minecraft.tags.TagKey;
import net.minecraft.util.RandomSource;
import net.minecraft.util.TickThrottler;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityEquipment;
import net.minecraft.world.entity.EntityFluidInteraction;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.InsideBlockEffectApplier;
import net.minecraft.world.entity.InterpolationHandler;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.entity.ai.attributes.Attribute;
import net.minecraft.world.entity.ai.attributes.AttributeMap;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.ai.attributes.DefaultAttributes;
import net.minecraft.world.entity.animal.equine.Horse;
import net.minecraft.world.entity.animal.pig.Pig;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.food.FoodData;
import net.minecraft.world.item.ItemCooldowns;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

/**
 * Runs real 26.3 ClientPacketListener (handleGameEvent CHANGE_GAME_MODE, handlePlayerAbilities,
 * handlePlayerInfoUpdate), MultiPlayerGameMode (adjustPlayer, setLocalMode, getPlayerMode,
 * getPreviousPlayerMode, isSpectator), AbstractClientPlayer (gameMode, getPlayerInfo),
 * Player (isSpectator, isCreative, tick), and LocalPlayer (aiStep, onUpdateAbilities) over a solid
 * stone floor at y <= 63, recording exact state and float/double bits for the gamemode split.
 */
public final class VanillaClientGameModeSplitOracle {
    static final UUID SELF_UUID = UUID.fromString("df71dcc7-da3b-3506-b229-6d1f589e2569");
    static final String SELF_NAME = "BX0W";
    static Object unsafe;
    static Method allocate;

    static <T> T make(Class<T> type) throws Exception {
        return type.cast(allocate.invoke(unsafe, type));
    }

    static Field field(Class<?> type, String name) throws Exception {
        Field f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static String f32Bits(float v) {
        return String.format("%08x", Float.floatToRawIntBits(v));
    }

    static String f64Bits(double v) {
        return String.format("%016x", Double.doubleToRawLongBits(v));
    }

    static String modeName(GameType mode) {
        return mode == null ? null : mode.getName();
    }

    static final class StoneFloorLevel extends ClientLevel {
        WorldBorder border;

        StoneFloorLevel() {
            super(null, null, null, null, 0, 0, null, false, 0L, 0);
        }

        @Override public boolean isClientSide() { return true; }
        @Override public BlockState getBlockState(BlockPos pos) {
            return pos.getY() <= 63 ? Blocks.STONE.defaultBlockState() : Blocks.AIR.defaultBlockState();
        }
        @Override public FluidState getFluidState(BlockPos pos) { return Fluids.EMPTY.defaultFluidState(); }
        @Override public boolean hasChunk(int x, int z) { return true; }
        @Override public int getMinY() { return -64; }
        @Override public int getHeight() { return 384; }
        @Override public WorldBorder getWorldBorder() { return border; }
        @Override public net.minecraft.world.level.BlockGetter getChunkForCollisions(int chunkX, int chunkZ) { return this; }
        @Override public List<Entity> getEntities(Entity e, AABB b) { return List.of(); }
        @Override public List<Entity> getEntities(Entity e, AABB b, java.util.function.Predicate<? super Entity> p) { return List.of(); }
        @Override public List<Entity> getPushableEntities(Entity e, AABB b) { return List.of(); }
    }

    static final class RecordingListener extends ClientPacketListener {
        List<Boolean> sentAbilities;

        RecordingListener() {
            super(null, null, null);
        }

        @Override public boolean hasClientLoaded() { return true; }

        @Override public void send(Packet<?> packet) {
            if (packet instanceof ServerboundPlayerAbilitiesPacket p) {
                sentAbilities.add(p.isFlying());
            }
        }
    }

    static final class SilentSocialManager extends PlayerSocialManager {
        SilentSocialManager() { super(null, null, null, null); }
        @Override public void addPlayer(PlayerInfo info) {}
        @Override public void removePlayer(UUID id) {}
    }

    static final class RealLocalPlayer extends LocalPlayer {
        AttributeMap attrs;
        byte sharedFlags;
        ClientAvatarState avState;
        boolean eyeInWater;

        RealLocalPlayer() {
            super(null, null, null, null, null, Input.EMPTY, false, null, null);
        }

        void configure(Level l) { setLevel(l); }
        InterpolationHandler makeInterp() { return createInterpolationHandler(); }
        @SuppressWarnings("unchecked")
        net.minecraft.network.syncher.SynchedEntityData makeEntityData() throws Exception {
            var builder = new net.minecraft.network.syncher.SynchedEntityData.Builder(this);
            builder.define((net.minecraft.network.syncher.EntityDataAccessor<Byte>) field(Entity.class, "DATA_SHARED_FLAGS_ID").get(null), (byte) 0);
            builder.define((net.minecraft.network.syncher.EntityDataAccessor<Integer>) field(Entity.class, "DATA_AIR_SUPPLY_ID").get(null), this.getMaxAirSupply());
            builder.define((net.minecraft.network.syncher.EntityDataAccessor<Boolean>) field(Entity.class, "DATA_CUSTOM_NAME_VISIBLE").get(null), false);
            builder.define((net.minecraft.network.syncher.EntityDataAccessor<Optional<net.minecraft.network.chat.Component>>) field(Entity.class, "DATA_CUSTOM_NAME").get(null), Optional.empty());
            builder.define((net.minecraft.network.syncher.EntityDataAccessor<Boolean>) field(Entity.class, "DATA_SILENT").get(null), false);
            builder.define((net.minecraft.network.syncher.EntityDataAccessor<Boolean>) field(Entity.class, "DATA_NO_GRAVITY").get(null), false);
            builder.define((net.minecraft.network.syncher.EntityDataAccessor<Pose>) field(Entity.class, "DATA_POSE").get(null), Pose.STANDING);
            builder.define((net.minecraft.network.syncher.EntityDataAccessor<Integer>) field(Entity.class, "DATA_TICKS_FROZEN").get(null), 0);
            defineSynchedData(builder);
            return builder.build();
        }

        // Keep AbstractClientPlayer.gameMode(), AbstractClientPlayer.getPlayerInfo(),
        // Player.isSpectator(), Player.isCreative(), Player.tick(), LocalPlayer.aiStep(),
        // and LocalPlayer.onUpdateAbilities() completely untouched!
        @Override public boolean isSleeping() { return false; }
        @Override public boolean onClimbable() { return false; }
        @Override public boolean tryToStartFallFlying() { return false; }
        @Override public boolean isEyeInFluid(TagKey<Fluid> tag) { return eyeInWater && tag == FluidTags.WATER; }
        @Override protected boolean updateIsUnderwater() { return this.wasUnderwater = this.isEyeInFluid(FluidTags.WATER); }
        @Override public float getEffectBlendFactor(Holder<MobEffect> effect, float partial) { return 0; }
        @Override public boolean isInWater() { return false; }
        @Override public boolean isInLava() { return false; }
        @Override public boolean isFallFlying() { return false; }
        @Override public boolean isUsingItem() { return false; }
        @Override public boolean isSprinting() { return false; }
        @Override public void setSprinting(boolean v) {}
        @Override public boolean isMobilityRestricted() { return false; }
        @Override public boolean isInShallowWater() { return false; }
        @Override public boolean isUnderWater() { return false; }
        @Override public boolean isMovingSlowly() { return false; }
        @Override public Pose getPose() { return Pose.STANDING; }
        @Override public void setPose(Pose p) {}
        @Override protected boolean isControlledCamera() { return false; }
        @Override public AttributeMap getAttributes() { return attrs; }
        @Override public boolean isAlive() { return true; }
        @Override public float getHealth() { return 20.0F; }
        @Override public boolean isNoGravity() { return false; }
        @Override public boolean isEffectiveAi() { return true; }
        @Override public boolean isLocalPlayer() { return true; }
        @Override protected boolean getSharedFlag(int bit) { return (sharedFlags & (1 << bit)) != 0; }
        @Override protected void setSharedFlag(int bit, boolean v) {
            sharedFlags = (byte) (v ? sharedFlags | (1 << bit) : sharedFlags & ~(1 << bit));
        }
        @Override public void updateAutoJump(float dx, float dz) {}
        @Override protected void applyEffectsFromBlocks() {}
        @Override public void handleShoulderEntities() {}
        @Override protected void pushEntities() {}
        @Override public void calculateEntityAnimation(boolean v) {}
        @Override public ClientAvatarState avatarState() { return avState; }
        @Override public ItemStack getItemBySlot(EquipmentSlot slot) { return ItemStack.EMPTY; }
        @Override public ItemStack getMainHandItem() { return ItemStack.EMPTY; }
        @Override protected void checkFallDamage(double ya, boolean onGround, BlockState onState, BlockPos pos) {}
        @Override public void checkDespawn() {}
        @Override protected void playStepSound(BlockPos pos, BlockState state) {}
        @Override public Optional<BlockPos> getSleepingPos() { return Optional.empty(); }
        @Override public int getTicksFrozen() { return 0; }
        @Override public void setTicksFrozen(int v) {}
        @Override public boolean isShiftKeyDown() { return false; }
        @Override protected boolean updateFluidInteraction() { return false; }
    }

    static final class SaddledHorse extends Horse {
        float maxHealth;
        float health;

        SaddledHorse() { super(null, null); }
        @Override public boolean isSaddled() { return true; }
        @Override public double getAttributeValue(Holder<Attribute> a) {
            return a == Attributes.MAX_HEALTH ? maxHealth : 0.0;
        }
        @Override public float getHealth() { return health; }
    }

    static final class SimplePig extends Pig {
        float maxHealth;
        float health;

        SimplePig() { super(null, null); }
        @Override public double getAttributeValue(Holder<Attribute> a) {
            return a == Attributes.MAX_HEALTH ? maxHealth : 0.0;
        }
        @Override public float getHealth() { return health; }
    }

    static final class SimpleRemotePlayer extends RemotePlayer {
        SimpleRemotePlayer() { super(null, null); }
    }

    static final class Harness {
        final Minecraft mc;
        final Options options;
        final StoneFloorLevel level;
        final RecordingListener listener;
        final Map<UUID, PlayerInfo> infoMap;
        final Camera mainCamera;
        final Hud hud;
        MultiPlayerGameMode gameMode;
        RealLocalPlayer player;

        Harness() throws Exception {
            options = make(Options.class);
            String[] keys = {"keyUp", "keyDown", "keyLeft", "keyRight", "keyJump", "keyShift", "keySprint"};
            for (String n : keys) {
                field(Options.class, n).set(options, make(KeyMapping.class));
            }
            var sprintWin = make(OptionInstance.class);
            field(OptionInstance.class, "value").set(sprintWin, 7);
            field(Options.class, "sprintWindow").set(options, sprintWin);

            mc = make(Minecraft.class);
            field(Minecraft.class, "instance").set(null, mc);
            field(Minecraft.class, "options").set(mc, options);
            field(Minecraft.class, "packetProcessor").set(mc, new PacketProcessor(Thread.currentThread()));
            var gui = make(Gui.class);
            field(Gui.class, "screen").set(gui, make(LevelLoadingScreen.class));
            field(Minecraft.class, "gui").set(mc, gui);
            field(Minecraft.class, "tutorial").set(mc, make(Tutorial.class));
            field(Minecraft.class, "playerSocialManager").set(mc, make(SilentSocialManager.class));
            var services = new net.minecraft.server.Services(
                null, com.mojang.authlib.services.ServicesKeySet.EMPTY, null, null, null);
            field(Minecraft.class, "services").set(mc, services);
            mainCamera = make(Camera.class);
            var renderer = make(GameRenderer.class);
            field(GameRenderer.class, "mainCamera").set(renderer, mainCamera);
            field(Minecraft.class, "gameRenderer").set(mc, renderer);
            hud = make(Hud.class);
            field(Hud.class, "minecraft").set(hud, mc);

            level = make(StoneFloorLevel.class);
            level.border = new WorldBorder();
            field(Level.class, "random").set(level, RandomSource.create(0L));
            mc.level = level;

            listener = make(RecordingListener.class);
            listener.sentAbilities = new ArrayList<>();
            field(ClientCommonPacketListenerImpl.class, "minecraft").set(listener, mc);
            infoMap = new HashMap<>();
            field(ClientPacketListener.class, "playerInfoMap").set(listener, infoMap);
            field(ClientPacketListener.class, "listedPlayers").set(listener, new java.util.HashSet<PlayerInfo>());
            field(ClientPacketListener.class, "waypointManager").set(listener, new ClientWaypointManager());

            gameMode = new MultiPlayerGameMode(mc, listener);
            mc.gameMode = gameMode;
            spawnPlayer(0.5, 64.0, 0.5, false);
        }

        void spawnPlayer(double x, double y, double z, boolean onGround) throws Exception {
            RealLocalPlayer p = make(RealLocalPlayer.class);
            p.configure(level);
            p.attrs = new AttributeMap(DefaultAttributes.getSupplier(EntityTypes.PLAYER));
            p.avState = new ClientAvatarState();
            p.setUUID(SELF_UUID);
            Vec3 pos = new Vec3(x, y, z);
            field(Entity.class, "position").set(p, pos);
            field(Entity.class, "xo").setDouble(p, x);
            field(Entity.class, "yo").setDouble(p, y);
            field(Entity.class, "zo").setDouble(p, z);
            BlockPos bp = BlockPos.containing(pos);
            field(Entity.class, "blockPosition").set(p, bp);
            field(Entity.class, "chunkPosition").set(p, new net.minecraft.world.level.ChunkPos(bp.getX() >> 4, bp.getZ() >> 4));
            field(Entity.class, "levelCallback").set(p, net.minecraft.world.level.entity.EntityInLevelCallback.NULL);
            field(Entity.class, "deltaMovement").set(p, Vec3.ZERO);
            field(Entity.class, "dimensions").set(p, EntityTypes.PLAYER.getDimensions());
            field(Entity.class, "bb").set(p, EntityTypes.PLAYER.getDimensions().makeBoundingBox(pos));
            field(Entity.class, "onGround").setBoolean(p, onGround);
            field(LivingEntity.class, "activeEffects").set(p, new HashMap<>());
            field(LivingEntity.class, "swingState").set(p, new LivingEntity.SwingState());
            field(LivingEntity.class, "elytraAnimationState").set(p, new net.minecraft.world.entity.ElytraAnimationState(p));
            field(LivingEntity.class, "walkAnimation").set(p, new net.minecraft.world.entity.WalkAnimationState());
            field(Entity.class, "passengers").set(p, com.google.common.collect.ImmutableList.of());
            field(Entity.class, "movementThisTick").set(p, new ArrayDeque<>());
            field(Entity.class, "mainSupportingBlockPos").set(p, Optional.empty());
            field(Entity.class, "stuckSpeedMultiplier").set(p, Vec3.ZERO);
            field(Entity.class, "random").set(p, RandomSource.create(0L));
            field(Entity.class, "type").set(p, EntityTypes.PLAYER);
            field(Entity.class, "entityData").set(p, p.makeEntityData());
            field(Entity.class, "interpolationHandler").set(p, p.makeInterp());
            field(LivingEntity.class, "equipment").set(p, new EntityEquipment());
            field(Entity.class, "finalMovementsThisTick").set(p, new ObjectArrayList<>());
            field(Entity.class, "visitedBlocks").set(p, new LongOpenHashSet());
            field(Entity.class, "insideEffectCollector").set(p, new InsideBlockEffectApplier.StepBasedCollector());
            field(Entity.class, "fluidInteraction").set(p, new EntityFluidInteraction(Set.of()));
            field(AbstractClientPlayer.class, "clientAvatarState").set(p, p.avState);
            field(Player.class, "abilities").set(p, new Abilities());
            field(Player.class, "inventory").set(p, new Inventory(p, new EntityEquipment()));
            field(Player.class, "foodData").set(p, new FoodData());
            field(Player.class, "cooldowns").set(p, new ItemCooldowns());
            field(Player.class, "lastItemInMainHand").set(p, ItemStack.EMPTY);
            field(LocalPlayer.class, "minecraft").set(p, mc);
            field(LocalPlayer.class, "connection").set(p, listener);
            field(LocalPlayer.class, "dropSpamThrottler").set(p, new TickThrottler(20, 1280));
            field(LocalPlayer.class, "firstPersonHandsAndItems").set(p, new FirstPersonHandsAndItems());
            field(LocalPlayer.class, "ambientSoundHandlers").set(p, new ArrayList<>());
            p.input = new KeyboardInput(options);
            this.player = p;
            mc.player = p;
            field(Camera.class, "entity").set(mainCamera, p);
        }

        void handleLogin(GameType gameType, GameType prev) throws Exception {
            gameMode = new MultiPlayerGameMode(mc, listener);
            mc.gameMode = gameMode;
            spawnPlayer(0.5, 64.0, 0.5, false);
            gameMode.adjustPlayer(player);
            gameMode.setLocalMode(gameType, prev);
        }

        void handleLoginRetainingPlayer(GameType gameType, GameType prev) {
            gameMode = new MultiPlayerGameMode(mc, listener);
            mc.gameMode = gameMode;
            gameMode.adjustPlayer(player);
            gameMode.setLocalMode(gameType, prev);
        }

        void handleRespawn(GameType gameType, GameType prev) throws Exception {
            spawnPlayer(0.5, 64.0, 0.5, false);
            gameMode.adjustPlayer(player);
            gameMode.setLocalMode(gameType, prev);
        }

        void handleRespawnKeepEntityData(GameType gameType, GameType prev) throws Exception {
            Vec3 oldVel = player.getDeltaMovement();
            spawnPlayer(0.5, 64.0, 0.5, false);
            player.setDeltaMovement(oldVel);
            gameMode.adjustPlayer(player);
            gameMode.setLocalMode(gameType, prev);
        }

        void handlePlayerInfoRemove(UUID id) {
            listener.handlePlayerInfoRemove(new ClientboundPlayerInfoRemovePacket(List.of(id)));
        }

        void handleChangeGameMode(float param) {
            listener.handleGameEvent(new ClientboundGameEventPacket(ClientboundGameEventPacket.CHANGE_GAME_MODE, param));
        }

        void handlePlayerAbilities(int flags, float flySpeed, float walkSpeed) {
            Abilities a = new Abilities();
            a.invulnerable = (flags & 1) != 0;
            a.flying = (flags & 2) != 0;
            a.mayfly = (flags & 4) != 0;
            a.instabuild = (flags & 8) != 0;
            a.setFlyingSpeed(flySpeed);
            a.setWalkingSpeed(walkSpeed);
            listener.handlePlayerAbilities(new ClientboundPlayerAbilitiesPacket(a));
        }

        void handlePlayerInfoAddAndMode(GameType mode) throws Exception {
            var entry = new ClientboundPlayerInfoUpdatePacket.Entry(
                SELF_UUID, new GameProfile(SELF_UUID, SELF_NAME), true, 0, mode, null, false, 0, null);
            var pkt = make(ClientboundPlayerInfoUpdatePacket.class);
            field(ClientboundPlayerInfoUpdatePacket.class, "actions").set(pkt, EnumSet.of(
                ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER,
                ClientboundPlayerInfoUpdatePacket.Action.UPDATE_GAME_MODE,
                ClientboundPlayerInfoUpdatePacket.Action.UPDATE_LISTED));
            field(ClientboundPlayerInfoUpdatePacket.class, "entries").set(pkt, List.of(entry));
            listener.handlePlayerInfoUpdate(pkt);
        }

        void handlePlayerInfoMode(GameType mode) throws Exception {
            var entry = new ClientboundPlayerInfoUpdatePacket.Entry(
                SELF_UUID, null, false, 0, mode, null, false, 0, null);
            var pkt = make(ClientboundPlayerInfoUpdatePacket.class);
            field(ClientboundPlayerInfoUpdatePacket.class, "actions").set(pkt, EnumSet.of(
                ClientboundPlayerInfoUpdatePacket.Action.UPDATE_GAME_MODE));
            field(ClientboundPlayerInfoUpdatePacket.class, "entries").set(pkt, List.of(entry));
            listener.handlePlayerInfoUpdate(pkt);
        }

        Map<String, Object> snapshot(String label) {
            Abilities a = player.getAbilities();
            PlayerInfo info = listener.getPlayerInfo(SELF_UUID);
            Vec3 pos = player.position();
            Vec3 vel = player.getDeltaMovement();
            Map<String, Object> ab = new LinkedHashMap<>();
            ab.put("invulnerable", a.invulnerable);
            ab.put("flying", a.flying);
            ab.put("may_fly", a.mayfly);
            ab.put("instabuild", a.instabuild);
            ab.put("may_build", a.mayBuild);
            ab.put("fly_speed_bits", f32Bits(a.getFlyingSpeed()));
            ab.put("walk_speed_bits", f32Bits(a.getWalkingSpeed()));

            Map<String, Object> row = new LinkedHashMap<>();
            row.put("step", label);
            row.put("local_game_mode", modeName(gameMode.getPlayerMode()));
            row.put("previous_local_game_mode", modeName(gameMode.getPreviousPlayerMode()));
            row.put("player_info_game_mode", info == null ? null : modeName(info.getGameMode()));
            row.put("player_game_mode", modeName(player.gameMode()));
            row.put("is_spectator", player.isSpectator());
            row.put("is_creative", player.isCreative());
            row.put("no_physics", player.noPhysics);
            row.put("on_ground", player.onGround());
            row.put("abilities", ab);
            row.put("pos_bits", List.of(f64Bits(pos.x), f64Bits(pos.y), f64Bits(pos.z)));
            row.put("vel_bits", List.of(f64Bits(vel.x), f64Bits(vel.y), f64Bits(vel.z)));
            row.put("sent_abilities", new ArrayList<>(listener.sentAbilities));
            listener.sentAbilities.clear();
            return row;
        }
    }

    static List<Map<String, Object>> scenarioGenpvpSequence() throws Exception {
        Harness h = new Harness();
        List<Map<String, Object>> steps = new ArrayList<>();
        h.handleLogin(GameType.CREATIVE, null);
        steps.add(h.snapshot("login_creative"));
        h.handlePlayerAbilities(0x0d, 0.05F, 0.10F);
        steps.add(h.snapshot("abilities_0x0d"));
        h.handlePlayerInfoAddAndMode(GameType.CREATIVE);
        steps.add(h.snapshot("player_info_add_creative"));
        h.handlePlayerAbilities(0x00, 0.05F, 0.10F);
        steps.add(h.snapshot("abilities_0x00_pre"));
        h.handlePlayerInfoMode(GameType.ADVENTURE);
        steps.add(h.snapshot("player_info_adventure"));
        h.handleChangeGameMode(2.0F);
        steps.add(h.snapshot("game_event_adventure"));
        h.handlePlayerAbilities(0x00, 0.05F, 0.10F);
        steps.add(h.snapshot("abilities_0x00_adv"));
        h.handleChangeGameMode(1.0F);
        steps.add(h.snapshot("game_event_creative"));
        h.handlePlayerAbilities(0x0d, 0.0F, 0.0F);
        steps.add(h.snapshot("abilities_0x0d_zero"));
        h.handleChangeGameMode(3.0F);
        steps.add(h.snapshot("game_event_spectator"));
        h.handlePlayerAbilities(0x00, 0.0F, 0.0F);
        steps.add(h.snapshot("abilities_0x00_zero"));
        for (int t = 1; t <= 5; t++) {
            h.player.tick();
            steps.add(h.snapshot("tick_" + t));
        }
        return steps;
    }

    static List<Map<String, Object>> scenarioChangeGameModeAbilities() throws Exception {
        List<Map<String, Object>> cases = new ArrayList<>();

        // 1. Leaving Spectator with flying=false and custom speeds -> Creative keeps flying=false and speeds.
        {
            Harness h = new Harness();
            h.handleLogin(GameType.SPECTATOR, null);
            h.handlePlayerInfoAddAndMode(GameType.ADVENTURE);
            h.handlePlayerAbilities(0x00, 0.125F, 0.2F);
            h.handleChangeGameMode(1.0F);
            cases.add(h.snapshot("spectator_not_flying_to_creative_keeps_flying_false_and_speeds"));
        }

        // 2. Leaving Spectator while flying=true within 1 block of stone floor (y=64.5) -> Creative does NOT land or reset speed.
        {
            Harness h = new Harness();
            h.handleLogin(GameType.SPECTATOR, null);
            h.spawnPlayer(0.5, 64.5, 0.5, false);
            h.gameMode.adjustPlayer(h.player);
            h.handlePlayerAbilities(0x07, 0.125F, 0.2F);
            h.handleChangeGameMode(1.0F);
            cases.add(h.snapshot("spectator_flying_near_ground_to_creative_does_not_land"));
        }

        // 3. PlayerAbilities followed by CHANGE_GAME_MODE applies GameType.updatePlayerAbilities in order, preserving speeds.
        {
            Harness h = new Harness();
            h.handleLogin(GameType.SURVIVAL, null);
            h.handlePlayerAbilities(0x0f, 0.08F, 0.15F);
            h.handleChangeGameMode(0.0F);
            cases.add(h.snapshot("abilities_0x0f_then_change_survival"));
            h.handlePlayerAbilities(0x0f, 0.08F, 0.15F);
            h.handleChangeGameMode(2.0F);
            cases.add(h.snapshot("abilities_0x0f_then_change_adventure"));
            h.handleChangeGameMode(0.6F);
            cases.add(h.snapshot("change_fractional_0_6_creative"));
            h.handleChangeGameMode(1.6F);
            cases.add(h.snapshot("change_fractional_1_6_adventure"));
            h.handleChangeGameMode(2.6F);
            cases.add(h.snapshot("change_fractional_2_6_spectator"));
            h.handleChangeGameMode(4.0F);
            cases.add(h.snapshot("change_invalid_4_survival"));
            h.handleChangeGameMode(-1.0F);
            cases.add(h.snapshot("change_invalid_neg1_survival"));
        }

        // 4. CHANGE_GAME_MODE followed by PlayerAbilities preserves mayBuild.
        {
            Harness h = new Harness();
            h.handleLogin(GameType.SURVIVAL, null);
            h.handleChangeGameMode(2.0F);
            h.handlePlayerAbilities(0x0f, 0.07F, 0.12F);
            cases.add(h.snapshot("adventure_then_abilities_preserves_may_build_false"));
            h.handleChangeGameMode(0.0F);
            h.handlePlayerAbilities(0x00, 0.05F, 0.10F);
            cases.add(h.snapshot("survival_then_abilities_preserves_may_build_true"));
        }

        // 5. Setting the same local mode again keeps previousLocalPlayerMode unchanged.
        {
            Harness h = new Harness();
            h.handleLogin(GameType.SURVIVAL, null);
            h.handleChangeGameMode(1.0F);
            h.handleChangeGameMode(1.0F);
            cases.add(h.snapshot("repeat_same_mode_keeps_previous_mode"));
        }

        return cases;
    }

    static List<Map<String, Object>> scenarioSplitModePhysicsMatrix() throws Exception {
        List<Map<String, Object>> rows = new ArrayList<>();
        GameType[] modes = GameType.values();
        GameType[] infoModes = {null, GameType.SURVIVAL, GameType.CREATIVE, GameType.ADVENTURE, GameType.SPECTATOR};
        for (GameType local : modes) {
            for (GameType info : infoModes) {
                for (boolean mayFly : new boolean[]{false, true}) {
                    Harness h = new Harness();
                    h.handleLogin(local, null);
                    if (info != null) {
                        h.handlePlayerInfoAddAndMode(info);
                    }
                    h.spawnPlayer(0.5, 64.0, 0.5, true);
                    h.gameMode.adjustPlayer(h.player);
                    h.handlePlayerAbilities(mayFly ? 0x04 : 0x00, 0.05F, 0.10F);
                    for (int t = 0; t < 3; t++) {
                        h.player.tick();
                    }
                    String name = "local=" + local.getName()
                        + ",info=" + (info == null ? "none" : info.getName())
                        + ",mayfly=" + mayFly + ",flying_init=false";
                    rows.add(h.snapshot(name));
                }
            }
        }
        // Descending into stone floor while flying=true: pins LocalPlayer.aiStep line 929
        // (`if (onGround() && abilities.flying && !minecraft.gameMode.isSpectator())`).
        for (GameType local : modes) {
            for (GameType info : new GameType[]{GameType.ADVENTURE, GameType.SPECTATOR}) {
                Harness h = new Harness();
                h.handleLogin(local, null);
                h.handlePlayerInfoAddAndMode(info);
                h.spawnPlayer(0.5, 64.0, 0.5, false);
                h.gameMode.adjustPlayer(h.player);
                h.handlePlayerAbilities(0x06, 0.05F, 0.10F); // flying=true, mayfly=true
                h.player.setDeltaMovement(new Vec3(0.0, -0.1, 0.0));
                h.player.tick();
                String name = "landing_local=" + local.getName() + ",info=" + info.getName() + ",flying_init=true";
                rows.add(h.snapshot(name));
            }
        }
        return rows;
    }

    static List<Map<String, Object>> scenarioLivePlayerInfoTransitions() throws Exception {
        Harness h = new Harness();
        List<Map<String, Object>> steps = new ArrayList<>();
        h.handleLogin(GameType.SPECTATOR, null);
        h.handlePlayerAbilities(0x00, 0.0F, 0.0F);
        for (int t = 0; t < 3; t++) h.player.tick();
        steps.add(h.snapshot("no_player_info_3ticks"));

        h.handlePlayerInfoAddAndMode(GameType.ADVENTURE);
        for (int t = 0; t < 3; t++) h.player.tick();
        steps.add(h.snapshot("info_adventure_3ticks"));

        h.handlePlayerInfoMode(GameType.SPECTATOR);
        for (int t = 0; t < 3; t++) h.player.tick();
        steps.add(h.snapshot("info_spectator_3ticks"));

        h.player.setPos(0.5, 64.0, 0.5);
        h.player.setDeltaMovement(Vec3.ZERO);
        h.handlePlayerInfoMode(GameType.SURVIVAL);
        for (int t = 0; t < 3; t++) h.player.tick();
        steps.add(h.snapshot("info_survival_3ticks"));
        return steps;
    }

    static List<Map<String, Object>> scenarioLoginAndRespawnPreviousMode() throws Exception {
        Harness h = new Harness();
        List<Map<String, Object>> steps = new ArrayList<>();
        h.handleLogin(GameType.CREATIVE, GameType.ADVENTURE);
        steps.add(h.snapshot("login_creative_prev_adventure"));

        h.handleChangeGameMode(3.0F);
        h.handlePlayerAbilities(0x07, 0.125F, 0.2F);
        steps.add(h.snapshot("before_respawn_spectator"));

        // Respawn from local=SPECTATOR into CREATIVE with previous=SURVIVAL.
        // ClientPacketListener.handleRespawn creates a new LocalPlayer, calls adjustPlayer (old SPECTATOR
        // sets flying=true on the fresh Abilities), then setLocalMode(CREATIVE, SURVIVAL) (keeps flying=true!).
        h.handleRespawn(GameType.CREATIVE, GameType.SURVIVAL);
        steps.add(h.snapshot("respawn_creative_prev_survival_from_spectator"));

        // Respawn from local=CREATIVE into ADVENTURE with previous=null.
        h.handleRespawn(GameType.ADVENTURE, null);
        steps.add(h.snapshot("respawn_adventure_prev_null_from_creative"));
        return steps;
    }

    static List<Map<String, Object>> scenarioResolvedPlayerInfoRetention() throws Exception {
        Harness h = new Harness();
        List<Map<String, Object>> steps = new ArrayList<>();
        h.handleLogin(GameType.SPECTATOR, null);
        h.handlePlayerAbilities(0x00, 0.05F, 0.10F);
        h.player.tick();
        steps.add(h.snapshot("before_add_player"));

        h.handlePlayerInfoAddAndMode(GameType.SPECTATOR);
        h.player.tick();
        steps.add(h.snapshot("resolved_spectator"));

        h.handlePlayerInfoRemove(SELF_UUID);
        h.player.tick();
        steps.add(h.snapshot("after_remove_retains_spectator"));

        h.handlePlayerInfoAddAndMode(GameType.ADVENTURE);
        h.player.tick();
        steps.add(h.snapshot("after_readd_adventure_retains_spectator"));

        h.handleRespawn(GameType.ADVENTURE, null);
        h.player.tick();
        steps.add(h.snapshot("after_respawn_resolves_adventure"));

        h.handleRespawn(GameType.SURVIVAL, null);
        h.handlePlayerInfoRemove(SELF_UUID);
        h.player.tick();
        steps.add(h.snapshot("after_respawn_and_remove_stays_unresolved"));

        h.handlePlayerInfoAddAndMode(GameType.CREATIVE);
        h.player.tick();
        steps.add(h.snapshot("after_respawn_readd_resolves_creative"));
        return steps;
    }

    static List<Map<String, Object>> scenarioWaterVisionTick() throws Exception {
        Field waterVisionTime = field(LocalPlayer.class, "waterVisionTime");
        Harness h = new Harness();
        List<Map<String, Object>> steps = new ArrayList<>();
        h.handleLogin(GameType.SPECTATOR, null);
        h.handlePlayerInfoAddAndMode(GameType.ADVENTURE);
        h.handlePlayerAbilities(0x06, 0.05F, 0.10F);

        record Case(String step, GameType local, GameType info, boolean eyeInWater) {}
        List<Case> cases = List.of(
            new Case("local_spectator_info_adventure_eye_water", GameType.SPECTATOR, GameType.ADVENTURE, true),
            new Case("local_adventure_info_spectator_eye_water", GameType.ADVENTURE, GameType.SPECTATOR, true),
            new Case("local_adventure_info_spectator_eye_air", GameType.ADVENTURE, GameType.SPECTATOR, false),
            new Case("local_spectator_info_spectator_eye_air_clamps_zero", GameType.SPECTATOR, GameType.SPECTATOR, false)
        );
        for (Case tc : cases) {
            h.handleChangeGameMode((float) tc.local().getId());
            h.handlePlayerInfoMode(tc.info());
            h.player.eyeInWater = tc.eyeInWater();
            h.player.tick();
            Map<String, Object> row = new LinkedHashMap<>();
            row.put("step", tc.step());
            row.put("local_game_mode", modeName(h.gameMode.getPlayerMode()));
            row.put("player_game_mode", modeName(h.player.gameMode()));
            row.put("eye_in_water", tc.eyeInWater());
            row.put("water_vision_time", waterVisionTime.getInt(h.player));
            steps.add(row);
        }
        return steps;
    }

    static List<Map<String, Object>> scenarioRespawnKeepDataSpectatorClearsY() throws Exception {
        Harness h = new Harness();
        List<Map<String, Object>> steps = new ArrayList<>();
        h.handleLogin(GameType.SURVIVAL, null);
        h.handlePlayerInfoAddAndMode(GameType.SURVIVAL);
        h.player.setDeltaMovement(new Vec3(0.25, 0.42, -0.1));
        h.handleRespawnKeepEntityData(GameType.SURVIVAL, null);
        steps.add(h.snapshot("after_respawn_keep_data"));

        h.handlePlayerInfoMode(GameType.SPECTATOR);
        steps.add(h.snapshot("after_player_info_spectator"));
        return steps;
    }

    static List<Map<String, Object>> scenarioRetainedPlayerLoginKeepsSpeeds() throws Exception {
        Harness h = new Harness();
        List<Map<String, Object>> steps = new ArrayList<>();
        h.handleLogin(GameType.CREATIVE, null);
        h.handlePlayerInfoAddAndMode(GameType.SPECTATOR);
        h.handlePlayerAbilities(0x0f, 0.125F, 0.2F);
        steps.add(h.snapshot("before_second_login"));

        h.handleLoginRetainingPlayer(GameType.SURVIVAL, GameType.CREATIVE);
        steps.add(h.snapshot("after_second_login_survival"));

        h.handleLoginRetainingPlayer(GameType.CREATIVE, GameType.SURVIVAL);
        steps.add(h.snapshot("after_third_login_creative"));
        return steps;
    }

    static SaddledHorse makeHorse(Entity rider, float maxHealth, float health) throws Exception {
        SaddledHorse horse = make(SaddledHorse.class);
        horse.maxHealth = maxHealth;
        horse.health = health;
        field(Entity.class, "passengers").set(horse, com.google.common.collect.ImmutableList.of(rider));
        return horse;
    }

    static SimplePig makePig(Entity rider, float maxHealth, float health) throws Exception {
        SimplePig pig = make(SimplePig.class);
        pig.maxHealth = maxHealth;
        pig.health = health;
        field(Entity.class, "passengers").set(
            pig, rider == null ? com.google.common.collect.ImmutableList.of() : com.google.common.collect.ImmutableList.of(rider));
        return pig;
    }

    static List<Map<String, Object>> scenarioHudCameraMounts() throws Exception {
        Method getCameraPlayer = Hud.class.getDeclaredMethod("getCameraPlayer");
        Method getVehicleWithHealth = Hud.class.getDeclaredMethod("getPlayerVehicleWithHealth");
        Method getVehicleMaxHearts = Hud.class.getDeclaredMethod("getVehicleMaxHearts", LivingEntity.class);
        Method nextContextualInfoState = Hud.class.getDeclaredMethod("nextContextualInfoState");
        getCameraPlayer.setAccessible(true);
        getVehicleWithHealth.setAccessible(true);
        getVehicleMaxHearts.setAccessible(true);
        nextContextualInfoState.setAccessible(true);

        List<Map<String, Object>> rows = new ArrayList<>();
        Harness h = new Harness();
        h.handleLogin(GameType.SPECTATOR, null);
        h.handlePlayerInfoAddAndMode(GameType.ADVENTURE);

        SaddledHorse localHorse = makeHorse(h.player, 30.0F, 24.0F);
        ArmorStand stand = make(ArmorStand.class);
        SimplePig freePig = makePig(null, 10.0F, 10.0F);
        SimpleRemotePlayer remoteUnmounted = make(SimpleRemotePlayer.class);
        SimpleRemotePlayer remoteOnPig = make(SimpleRemotePlayer.class);
        SimplePig remotePig = makePig(remoteOnPig, 10.0F, 8.0F);
        field(Entity.class, "vehicle").set(remoteOnPig, remotePig);
        SimpleRemotePlayer remoteOnHorse = make(SimpleRemotePlayer.class);
        SaddledHorse remoteHorse = makeHorse(remoteOnHorse, 30.0F, 20.0F);
        field(Entity.class, "vehicle").set(remoteOnHorse, remoteHorse);

        record Case(String step, GameType mode, Entity localMount, Entity camera, String cameraKind) {}
        List<Case> cases = List.of(
            new Case("spectator_local_mounted_camera_self", GameType.SPECTATOR, localHorse, h.player, "local"),
            new Case("spectator_local_mounted_camera_armor_stand", GameType.SPECTATOR, localHorse, stand, "non_player"),
            new Case("spectator_local_mounted_camera_pig", GameType.SPECTATOR, localHorse, freePig, "non_player"),
            new Case("survival_local_mounted_camera_armor_stand", GameType.SURVIVAL, localHorse, stand, "non_player"),
            new Case("spectator_local_mounted_camera_remote_unmounted", GameType.SPECTATOR, localHorse, remoteUnmounted, "remote_player"),
            new Case("spectator_local_mounted_camera_remote_mounted_pig", GameType.SPECTATOR, localHorse, remoteOnPig, "remote_player"),
            new Case("spectator_local_unmounted_camera_remote_mounted_horse", GameType.SPECTATOR, null, remoteOnHorse, "remote_player")
        );
        for (Case tc : cases) {
            h.handleChangeGameMode((float) tc.mode().getId());
            field(Entity.class, "vehicle").set(h.player, tc.localMount());
            field(Camera.class, "entity").set(h.mainCamera, tc.camera());
            Player cameraPlayer = (Player) getCameraPlayer.invoke(h.hud);
            LivingEntity vehicle = (LivingEntity) getVehicleWithHealth.invoke(h.hud);
            int hearts = (int) getVehicleMaxHearts.invoke(h.hud, vehicle);
            Object contextual = nextContextualInfoState.invoke(h.hud);
            Map<String, Object> row = new LinkedHashMap<>();
            row.put("step", tc.step());
            row.put("local_game_mode", modeName(h.gameMode.getPlayerMode()));
            row.put("camera_kind", tc.cameraKind());
            row.put("camera_is_player", cameraPlayer != null);
            row.put("vehicle_max_hearts", hearts);
            row.put("vehicle_health_bits", vehicle == null ? null : f32Bits(vehicle.getHealth()));
            row.put("contextual_info", contextual.toString().toLowerCase(Locale.ROOT));
            rows.add(row);
        }
        return rows;
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
        unsafe = field(unsafeClass, "theUnsafe").get(null);
        allocate = unsafeClass.getMethod("allocateInstance", Class.class);

        Map<String, Object> root = new LinkedHashMap<>();
        root.put("version", "26.3");
        root.put("java_version", System.getProperty("java.version"));
        root.put("oracle", "VanillaClientGameModeSplitOracle");

        Map<String, Object> scenarios = new LinkedHashMap<>();
        scenarios.put("genpvp_packet_sequence", scenarioGenpvpSequence());
        scenarios.put("change_game_mode_abilities", scenarioChangeGameModeAbilities());
        scenarios.put("split_mode_physics_matrix", scenarioSplitModePhysicsMatrix());
        scenarios.put("live_player_info_transitions", scenarioLivePlayerInfoTransitions());
        scenarios.put("login_and_respawn_previous_mode", scenarioLoginAndRespawnPreviousMode());
        scenarios.put("resolved_player_info_retention", scenarioResolvedPlayerInfoRetention());
        scenarios.put("water_vision_tick", scenarioWaterVisionTick());
        scenarios.put("respawn_keep_data_spectator_clears_y", scenarioRespawnKeepDataSpectatorClearsY());
        scenarios.put("retained_player_login_keeps_speeds", scenarioRetainedPlayerLoginKeepsSpeeds());
        scenarios.put("hud_camera_mounts", scenarioHudCameraMounts());
        root.put("scenarios", scenarios);

        Path out = Path.of(args[0]);
        Files.createDirectories(out.getParent());
        Gson gson = new GsonBuilder().setPrettyPrinting().serializeNulls().create();
        try (Writer w = Files.newBufferedWriter(out, StandardCharsets.UTF_8)) {
            gson.toJson(root, w);
            w.write('\n');
        }
    }
}
