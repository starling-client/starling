package dev.sunriselab.bench.mixin;

import dev.sunriselab.bench.VanillaBench;

import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Times the client tick (Minecraft.tick), for the stress scenes' client tick figure. */
@Mixin(Minecraft.class)
public abstract class TickHookMixin {
    @Inject(method = "tick", at = @At("HEAD"))
    private void sunriseBenchTickStart(CallbackInfo callback) {
        VanillaBench.tick(true);
    }

    @Inject(method = "tick", at = @At("RETURN"))
    private void sunriseBenchTickEnd(CallbackInfo callback) {
        VanillaBench.tick(false);
    }
}
