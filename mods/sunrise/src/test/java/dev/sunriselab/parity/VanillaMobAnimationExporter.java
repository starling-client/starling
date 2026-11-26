package dev.sunriselab.parity;

import com.google.gson.*;
import java.lang.reflect.Modifier;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.zip.ZipFile;
import net.minecraft.SharedConstants;
import net.minecraft.client.animation.AnimationChannel;
import net.minecraft.client.animation.AnimationDefinition;
import net.minecraft.client.animation.Keyframe;
import net.minecraft.server.Bootstrap;

/** Exports keyframe animation definitions from the client jar's entity models. */
public final class VanillaMobAnimationExporter {
    private static JsonArray vector(org.joml.Vector3fc v) {
        var a = new JsonArray();
        a.add(v.x()); a.add(v.y()); a.add(v.z());
        return a;
    }

    private static JsonObject definition(AnimationDefinition d) {
        var o = new JsonObject();
        o.addProperty("length", d.lengthInSeconds());
        o.addProperty("looping", d.looping());
        var bones = new JsonObject();
        for (var entry : new TreeMap<>(d.boneAnimations()).entrySet()) {
            var channels = new JsonArray();
            for (AnimationChannel channel : entry.getValue()) {
                var c = new JsonObject();
                var target = channel.target();
                c.addProperty("target", target == AnimationChannel.Targets.POSITION ? "position"
                    : target == AnimationChannel.Targets.ROTATION ? "rotation" : "scale");
                var frames = new JsonArray();
                for (Keyframe k : channel.keyframes()) {
                    var f = new JsonArray();
                    f.add(k.timestamp());
                    f.add(vector(k.preTarget()));
                    f.add(vector(k.postTarget()));
                    f.add(k.interpolation() == AnimationChannel.Interpolations.CATMULLROM ? "catmullrom" : "linear");
                    frames.add(f);
                }
                c.add("keyframes", frames);
                channels.add(c);
            }
            bones.add(entry.getKey(), channels);
        }
        o.add("bones", bones);
        return o;
    }

    private static JsonObject animations(String clientJar) throws Exception {
        var out = new JsonObject();
        var names = new TreeSet<String>();
        try (var zip = new ZipFile(clientJar)) {
            var entries = zip.entries();
            while (entries.hasMoreElements()) {
                var name = entries.nextElement().getName();
                if (name.startsWith("net/minecraft/client/animation/definitions/") && name.endsWith(".class") && !name.contains("$"))
                    names.add(name.substring(0, name.length() - 6).replace('/', '.'));
            }
        }
        for (var name : names) {
            var cls = Class.forName(name);
            for (var field : cls.getDeclaredFields()) {
                if (Modifier.isStatic(field.getModifiers()) && field.getType() == AnimationDefinition.class) {
                    field.setAccessible(true);
                    out.add(cls.getSimpleName() + "." + field.getName(), definition((AnimationDefinition) field.get(null)));
                }
            }
        }
        return out;
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Files.writeString(Path.of(args[0]), new Gson().toJson(animations(args[1])), StandardCharsets.UTF_8);
    }
}
