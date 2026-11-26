package dev.sunriselab.parity;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.UUID;
import net.minecraft.core.Holder;
import net.minecraft.core.component.DataComponentMap;
import net.minecraft.core.component.DataComponents;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.inventory.ContainerSynchronizer;
import net.minecraft.world.inventory.RemoteSlot;
import net.minecraft.world.damagesource.DamageSource;
import net.minecraft.world.damagesource.DamageType;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityEquipment;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.ai.Brain;
import net.minecraft.world.entity.ai.memory.NearestVisibleLivingEntities;
import net.minecraft.world.entity.ai.sensing.Sensing;
import net.minecraft.world.entity.ai.gossip.GossipContainer;
import net.minecraft.world.entity.npc.villager.Villager;
import net.minecraft.world.entity.npc.villager.VillagerData;
import net.minecraft.world.entity.npc.villager.AbstractVillager;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.MerchantMenu;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.component.ResolvableProfile;
import net.minecraft.world.item.trading.ItemCost;
import net.minecraft.world.item.trading.MerchantOffer;
import net.minecraft.world.item.trading.MerchantOffers;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.Vec3;

/** Actual lazy sensor visibility and hurt reputation on isolated entities; no server is constructed. */
public final class VanillaVillagerWitnessOracle {
    static Object unsafe;
    static Method allocate;
    static Field field(Class<?> type, String name) throws Exception {
        var f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }
    static <T> T make(Class<T> type) throws Exception {
        return type.cast(allocate.invoke(unsafe, type));
    }
    static final class Actor extends Villager {
        ServerLevel world;
        boolean invisible;
        Brain<Villager> brain;
        Sensing sensing;
        Actor() { super(EntityTypes.VILLAGER, null); }
        @Override public Level level() { return world; }
        @Override public boolean isAlive() { return true; }
        @Override public boolean isSpectator() { return false; }
        @Override public boolean isInvisible() { return invisible; }
        @Override public boolean isDiscrete() { return false; }
        @Override public float getArmorCoverPercentage() { return 0.0F; }
        @Override public ItemStack getItemBySlot(EquipmentSlot slot) { return ItemStack.EMPTY; }
        @Override public Brain<Villager> getBrain() { return brain; }
        @Override public Sensing getSensing() { return sensing; }
        @Override public boolean hasLineOfSight(Entity target) { return true; }
    }
    static Actor actor(ServerLevel world, int id, double x, double z, boolean invisible) throws Exception {
        var actor = make(Actor.class);
        actor.world = world;
        actor.invisible = invisible;
        actor.brain = new Brain<>();
        actor.sensing = new Sensing(actor);
        field(Entity.class, "id").setInt(actor, id);
        field(Entity.class, "position").set(actor, new Vec3(x, 1.0, z));
        return actor;
    }
    static String witnesses(NearestVisibleLivingEntities memory) {
        var ids = new ArrayList<Integer>();
        for (var entity : memory.findAll(e -> e instanceof Villager)) ids.add(entity.getId());
        return ids.toString().replace(" ", "");
    }
    static final class HurtLevel extends ServerLevel {
        List<String> trace;
        HurtLevel() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        @Override public long getGameTime() { return 1L; }
        @Override public void broadcastEntityEvent(Entity entity, byte event) { trace.add("entity:" + event); }
        @Override public void broadcastDamageEvent(Entity entity, DamageSource source) { trace.add("damage"); }
    }
    static final class HurtVillager extends Villager {
        HurtLevel world;
        float health;
        HurtVillager() { super(EntityTypes.VILLAGER, null); }
        @Override public HurtLevel level() { return world; }
        @Override public boolean isAlive() { return health > 0.0F; }
        @Override public float getHealth() { return health; }
        @Override public boolean isInvulnerableTo(ServerLevel level, DamageSource source) { return false; }
        @Override public boolean isSleeping() { return false; }
        @Override public boolean isUsingItem() { return false; }
        @Override public ItemStack getUseItem() { return ItemStack.EMPTY; }
        @Override public ItemStack getItemBySlot(EquipmentSlot slot) { return ItemStack.EMPTY; }
        @Override public Collection<MobEffectInstance> getActiveEffects() { return List.of(); }
        @Override public VillagerData getVillagerData() { return new VillagerData(null, null, 1); }
        // Leave the actual hurtServer cooldown, responsibility, broadcasts, gossip and prices
        // in vanilla. Supply only the local health and observe the sinks for knockback/sound.
        @Override protected void actuallyHurt(ServerLevel level, DamageSource source, float damage) { health -= damage; }
        @Override protected Player resolvePlayerResponsibleForDamage(DamageSource source) { return null; }
        @Override public void dealDefaultKnockback(DamageSource source, float amount, boolean blocked) {}
        @Override public float getVoicePitch() { return 1.0F; }
        @Override public void playSound(SoundEvent sound, float volume, float pitch) {
            if (sound == SoundEvents.VILLAGER_NO) world.trace.add("no");
            if (sound == SoundEvents.VILLAGER_HURT) world.trace.add("sound");
        }
        @Override public void die(DamageSource source) {}
    }
    static final class Buyer extends Player {
        HurtLevel world;
        Buyer() { super(null, null); }
        @Override public GameType gameMode() { return GameType.SURVIVAL; }
        @Override public ResolvableProfile getProfile() { throw new UnsupportedOperationException(); }
        @Override public MobEffectInstance getEffect(Holder<MobEffect> effect) { return null; }
        @Override public void sendMerchantOffers(int container, MerchantOffers offers, int level, int xp, boolean progress, boolean restock) {
            world.trace.add("offer:" + offers.getFirst().getCostA().getCount());
        }
    }
    static Buyer buyer(HurtLevel world, int identity) throws Exception {
        var player = make(Buyer.class);
        player.world = world;
        field(Entity.class, "id").setInt(player, identity);
        field(Entity.class, "uuid").set(player, new UUID(0L, identity));
        return player;
    }
    static List<String> hurt(boolean buyerFirst) throws Exception {
        var world = make(HurtLevel.class);
        world.trace = new ArrayList<>();
        var villager = make(HurtVillager.class);
        villager.world = world;
        villager.health = 20.0F;
        field(Villager.class, "gossips").set(villager, new GossipContainer());
        Items.AIR.builtInRegistryHolder().bindComponents(DataComponentMap.EMPTY);
        var stackable = DataComponentMap.builder().set(DataComponents.MAX_STACK_SIZE, 64).build();
        Items.EMERALD.builtInRegistryHolder().bindComponents(stackable);
        Items.GLASS.builtInRegistryHolder().bindComponents(stackable);
        var offers = new MerchantOffers();
        offers.add(new MerchantOffer(new ItemCost(Items.EMERALD, 4), new ItemStack(Items.GLASS, 3), 100, 1, 0.2F));
        field(AbstractVillager.class, "offers").set(villager, offers);
        var buyer = buyer(world, 2);
        var other = buyer(world, 3);
        var lethal = buyer(world, 5);
        var menu = new MerchantMenu(1, new Inventory(buyer, new EntityEquipment()), villager);
        buyer.containerMenu = menu;
        villager.setTradingPlayer(buyer);
        var ambient = field(net.minecraft.world.entity.Mob.class, "ambientSoundTime");
        ambient.setInt(villager, -80);
        menu.getSlot(0).set(new ItemStack(Items.EMERALD, 4));
        if (menu.getSlot(2).getItem().isEmpty()) throw new AssertionError("missing initial result");
        menu.setSynchronizer(new ContainerSynchronizer() {
            @Override public void sendInitialData(AbstractContainerMenu container, List<ItemStack> slots, ItemStack carried, int[] data) {}
            @Override public void sendSlotChange(AbstractContainerMenu container, int index, ItemStack stack) {
                if (index == 2) world.trace.add(stack.isEmpty() ? "result:empty" : "result:filled");
            }
            @Override public void sendCarriedChange(AbstractContainerMenu container, ItemStack carried) {}
            @Override public void sendDataChange(AbstractContainerMenu container, int id, int value) {}
            @Override public RemoteSlot createSlot() { return new RemoteSlot.Synchronized(null); }
        });
        ambient.setInt(villager, 0);
        var mob = actor(world, 1, 2, 0, false);
        var weak = actor(world, 4, 2, 0, false);
        field(Entity.class, "uuid").set(mob, new UUID(0L, 1));
        field(Entity.class, "uuid").set(weak, new UUID(0L, 4));
        var type = Holder.direct(new DamageType("test", 0.0F));
        if (!villager.hurtServer(world, new DamageSource(type, buyerFirst ? buyer : other), 1.0F)
                || !villager.hurtServer(world, new DamageSource(type, buyerFirst ? other : buyer), 3.0F)) {
            throw new AssertionError("same-tick player hits must both be accepted");
        }
        if (world.trace.stream().anyMatch(event -> event.startsWith("result:"))) {
            throw new AssertionError("the result was sent before the menu broadcast");
        }
        menu.broadcastChanges(); // ServerPlayer.tick, before stillValid.
        if (!villager.hurtServer(world, new DamageSource(type, mob), 5.0F)
                || villager.hurtServer(world, new DamageSource(type, weak), 2.0F)
                || !villager.hurtServer(world, new DamageSource(type, mob), 7.0F)
                || !villager.hurtServer(world, new DamageSource(type, lethal), 1000.0F)) {
            throw new AssertionError("same-tick damage acceptance changed");
        }
        var reps = new ArrayList<Integer>();
        for (int i = 1; i <= 5; i++) {
            reps.add(villager.getGossips().getReputation(new UUID(0L, i), t -> true));
        }
        return List.of(reps.toString().replace(" ", ""), String.join(",", world.trace));
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        var world = make(ServerLevel.class);
        var victim = actor(world, 1, 0, 0, false);
        var near = actor(world, 2, 10, 0, false);
        var hidden = actor(world, 3, 10, 0, true);
        var diagonal = actor(world, 4, 15, 15, false);
        var close = actor(world, 6, 1.5, 0, true);
        var far = actor(world, 7, 20, 0, false);
        List<LivingEntity> candidates = List.of(near, hidden, diagonal, close, far);
        var memory = new NearestVisibleLivingEntities(world, victim, candidates);
        try (var out = Files.newBufferedWriter(Path.of(args[0]))) {
            out.write("witnesses " + witnesses(memory) + "\n");
            near.invisible = true;
            hidden.invisible = false;
            out.write("cached " + witnesses(memory) + "\n");
            memory = new NearestVisibleLivingEntities(world, victim, candidates);
            out.write("fresh " + witnesses(memory) + "\n");
            var hurt = hurt(false);
            out.write("hurt " + hurt.get(0) + "\n");
            out.write("hurt_order " + hurt.get(1) + "\n");
            out.write("hurt_order_buyer " + hurt(true).get(1) + "\n");
        }
    }
}
