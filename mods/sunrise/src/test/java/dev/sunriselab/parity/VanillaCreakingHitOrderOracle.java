package dev.sunriselab.parity;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import io.netty.channel.ChannelHandlerContext;
import io.netty.channel.ChannelOutboundHandlerAdapter;
import io.netty.channel.ChannelPromise;
import io.netty.channel.embedded.EmbeddedChannel;
import java.lang.reflect.Field;
import java.util.UUID;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.SectionPos;
import net.minecraft.network.protocol.game.ClientboundEntityEventPacket;
import net.minecraft.network.protocol.game.ClientboundSoundEntityPacket;
import net.minecraft.network.protocol.game.ClientboundSoundPacket;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.monster.creaking.Creaking;
import net.minecraft.world.entity.projectile.arrow.AbstractArrow;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.entity.CreakingHeartBlockEntity;
import net.minecraft.world.level.gameevent.BlockPositionSource;
import net.minecraft.world.level.gameevent.GameEvent;
import net.minecraft.world.level.gameevent.GameEventListener;
import net.minecraft.world.level.gameevent.PositionSource;
import net.minecraft.world.phys.Vec3;

/** Observes live arrow and in-wall hurt reactions without replacing the Creaking's tick. */
final class VanillaCreakingHitOrderOracle {
    static JsonArray run(ServerPlayer player, EmbeddedChannel channel, JsonObject op) throws ReflectiveOperationException {
        ServerLevel level = player.level();
        if (!(level.getEntity(UUID.fromString(op.get("creaking").getAsString())) instanceof Creaking creaking)
                || creaking.getHomePos() == null
                || !(level.getBlockEntity(creaking.getHomePos()) instanceof CreakingHeartBlockEntity heart)
                || !heart.isProtector(creaking)) {
            throw new IllegalStateException("missing the live protector or its heart");
        }
        AbstractArrow arrow = null;
        if (op.has("arrow")) {
            if (!(level.getEntity(UUID.fromString(op.get("arrow").getAsString())) instanceof AbstractArrow hit)
                    || hit.getOwner() != null) {
                throw new IllegalStateException("missing the ownerless arrow");
            }
            arrow = hit;
        }
        Field animation = Creaking.class.getDeclaredField("invulnerabilityAnimationRemainingTicks");
        Field emitter = CreakingHeartBlockEntity.class.getDeclaredField("emitter");
        animation.setAccessible(true);
        emitter.setAccessible(true);
        if (arrow != null) {
            // Same precondition as the native case: a player hit has just started the emitter.
            if (emitter.getInt(heart) != 0 || !creaking.hurtServer(level, level.damageSources().playerAttack(player), 1.0F)
                    || emitter.getInt(heart) != 100) {
                throw new IllegalStateException("the player hit did not start the emitter synchronously");
            }
            animation.setInt(creaking, 0);
            arrow.setDeltaMovement(-1.0, 0.0, 0.0);
        }

        JsonArray actions = new JsonArray();
        JsonArray sounds = new JsonArray();
        int[] flashes = {0};
        var listener = new GameEventListener() {
            @Override public PositionSource getListenerSource() { return new BlockPositionSource(creaking.blockPosition()); }
            @Override public int getListenerRadius() { return 1; }
            @Override public boolean handleGameEvent(ServerLevel level, Holder<GameEvent> event, GameEvent.Context context, Vec3 pos) {
                if (event == GameEvent.ENTITY_ACTION && context.sourceEntity() == creaking) {
                    JsonArray at = new JsonArray();
                    at.add(flashes[0]);
                    at.add(sounds.size());
                    actions.add(at);
                    return true;
                }
                return false;
            }
        };
        var observer = new ChannelOutboundHandlerAdapter() {
            @Override public void write(ChannelHandlerContext ctx, Object packet, ChannelPromise promise) throws Exception {
                if (packet instanceof ClientboundEntityEventPacket event && event.getEntity(level) == creaking && event.getEventId() == 66) {
                    flashes[0]++;
                } else {
                    var event = packet instanceof ClientboundSoundEntityPacket sound ? sound.getSound()
                            : packet instanceof ClientboundSoundPacket sound ? sound.getSound() : null;
                    if (event != null) {
                        String id = event.value().location().toString();
                        if (id.equals("minecraft:entity.creaking.sway") || id.equals("minecraft:entity.arrow.hit")) {
                            sounds.add(id);
                        }
                    }
                }
                super.write(ctx, packet, promise);
            }
        };
        var listeners = level.getChunkAt(creaking.blockPosition()).getListenerRegistry(SectionPos.blockToSectionCoord(creaking.getBlockY()));
        listeners.register(listener);
        channel.pipeline().addLast(observer);
        try {
            JsonArray trace = new JsonArray();
            if (arrow != null) {
                level.tickNonPassenger(arrow);
                trace.add(snapshot(actions, flashes[0], sounds, emitter.getInt(heart), animation.getInt(creaking)));
                creaking.setNoAi(false);
                level.tickNonPassenger(creaking);
                trace.add(snapshot(actions, flashes[0], sounds, emitter.getInt(heart), animation.getInt(creaking)));
            } else {
                creaking.setLastHurtByPlayer(player, 1);
                level.tickNonPassenger(creaking);
                trace.add(credit(snapshot(actions, flashes[0], sounds, emitter.getInt(heart), animation.getInt(creaking)), creaking));
                level.setBlockAndUpdate(BlockPos.containing(creaking.getX(), creaking.getEyeY(), creaking.getZ()), Blocks.STONE.defaultBlockState());
                level.tickNonPassenger(creaking);
                trace.add(credit(snapshot(actions, flashes[0], sounds, emitter.getInt(heart), animation.getInt(creaking)), creaking));
                // With no cooldown and no remembered player, the same wall cannot react again.
                animation.setInt(creaking, 0);
                level.tickNonPassenger(creaking);
                trace.add(credit(snapshot(actions, flashes[0], sounds, emitter.getInt(heart), animation.getInt(creaking)), creaking));
            }
            return trace;
        } finally {
            channel.pipeline().remove(observer);
            listeners.unregister(listener);
        }
    }

    private static JsonObject credit(JsonObject result, Creaking creaking) {
        result.addProperty("remembered", creaking.getLastHurtByPlayer() != null);
        result.addProperty("clock", creaking.getLastHurtByPlayerMemoryTime());
        return result;
    }

    private static JsonObject snapshot(JsonArray action, int flashes, JsonArray sounds, int emitter, int animation) {
        JsonObject result = new JsonObject();
        result.add("action", action.deepCopy());
        result.addProperty("flashes", flashes);
        result.add("sounds", sounds.deepCopy());
        result.addProperty("emitter", emitter);
        result.addProperty("animation", animation);
        return result;
    }
}
