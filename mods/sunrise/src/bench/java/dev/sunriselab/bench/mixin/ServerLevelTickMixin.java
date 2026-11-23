package dev.sunriselab.bench.mixin;

import dev.sunriselab.bench.RedstoneDiff;
import java.util.function.BooleanSupplier;
import net.minecraft.server.level.ServerLevel;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Tells the redstone differential driver (RedstoneDiff) where a server level's tick begins and ends. */
@Mixin(ServerLevel.class)
public abstract class ServerLevelTickMixin {
    @Inject(method = "tick", at = @At("HEAD"))
    private void sunriseBenchLevelTickStart(BooleanSupplier haveTime, CallbackInfo callback) {
        RedstoneDiff.level((ServerLevel) (Object) this, true);
    }

    @Inject(method = "tick", at = @At("RETURN"))
    private void sunriseBenchLevelTickEnd(BooleanSupplier haveTime, CallbackInfo callback) {
        RedstoneDiff.level((ServerLevel) (Object) this, false);
    }
}
