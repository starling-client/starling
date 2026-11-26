package dev.sunriselab.parity;

import com.google.gson.Gson;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import net.minecraft.client.renderer.RenderPipelines;

/** Read the two registries NeoForge extends rather than maintaining a copy of their keys. */
public final class VanillaRenderPipelineExporter {
    private static List<String> locations(List<RenderPipeline> pipelines) {
        return pipelines.stream().map(pipeline -> pipeline.getLocation().toString()).sorted().toList();
    }

    public static void main(String[] args) throws Exception {
        // Map.of iterates in an order that changes from one JVM to the next; the file must not.
        var registries = new TreeMap<>(Map.of(
            "required", locations(RenderPipelines.requiredPipelines()),
            "optional", locations(RenderPipelines.optionalPipelines())
        ));
        Files.writeString(Path.of(args[0]), new Gson().toJson(registries));
    }
}
