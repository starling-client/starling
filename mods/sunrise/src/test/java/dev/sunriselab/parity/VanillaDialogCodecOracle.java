package dev.sunriselab.parity;

import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mojang.serialization.JsonOps;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import net.minecraft.ReportedException;
import net.minecraft.core.registries.Registries;
import net.minecraft.nbt.NbtOps;
import net.minecraft.nbt.TagParser;
import net.minecraft.network.chat.ClickEvent;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.contents.TranslatableContents;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.RegistryOps;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.dialog.Dialog;
import net.minecraft.server.dialog.NoticeDialog;
import net.minecraft.server.dialog.body.ItemBody;
import net.minecraft.server.dialog.input.TextInput;
import net.minecraft.server.packs.PackLocationInfo;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.PathPackResources;
import net.minecraft.server.packs.repository.PackSource;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagKey;
import net.minecraft.tags.TagLoader;

/**
 * Runs the real dialog codec on a corpus file ({"cases": [...], "packs": [...]}).
 * A case ({"name", "dialog"}) is decoded by Dialog.DIRECT_CODEC over the vanilla
 * data pack's registries; its line is {"name", "accept", "error"}, the error
 * being the codec's own message. A pack ({"name", "files": {path: json}}) is
 * written over the vanilla pack and loaded by RegistryDataLoader as a world is
 * opened; its line is {"name", "accept", "error"}, the error being the
 * "registry/id: message" lines of the load report. Packet cases exercise the
 * same direct codec over NbtOps, and selected JSON cases also encode and read
 * the NBT again before reporting their decoded values.
 */
public final class VanillaDialogCodecOracle {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var ops = RegistryOps.create(JsonOps.INSTANCE, ParityData.lookup);
        var nbtOps = RegistryOps.create(NbtOps.INSTANCE, ParityData.lookup);
        JsonObject corpus = JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonObject();
        Path scratch = Files.createTempDirectory("dialog-packs");
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            for (JsonElement entry : corpus.getAsJsonArray("cases")) {
                JsonObject one = entry.getAsJsonObject();
                var result = Dialog.DIRECT_CODEC.parse(ops, one.get("dialog"));
                JsonObject line = new JsonObject();
                line.addProperty("name", one.get("name").getAsString());
                line.addProperty("accept", result.result().isPresent());
                if (result.result().isEmpty()) line.addProperty("error", result.error().orElseThrow().message());
                if (one.has("observe")) {
                    var encoded = Dialog.DIRECT_CODEC.encodeStart(nbtOps, result.getOrThrow()).getOrThrow();
                    var decoded = Dialog.DIRECT_CODEC.parse(nbtOps, encoded).getOrThrow();
                    line.add("round_trip", observe(decoded, one.get("observe").getAsString()));
                }
                out.println(new Gson().toJson(line));
            }
            for (JsonElement entry : corpus.getAsJsonArray("packets")) {
                JsonObject one = entry.getAsJsonObject();
                var result = Dialog.DIRECT_CODEC.parse(nbtOps, TagParser.parseCompoundFully(one.get("snbt").getAsString()));
                JsonObject line = new JsonObject();
                line.addProperty("name", one.get("name").getAsString());
                line.addProperty("accept", result.result().isPresent());
                if (result.result().isEmpty()) line.addProperty("error", result.error().orElseThrow().message());
                if (one.has("observe")) {
                    line.add("round_trip", observe(result.getOrThrow(), one.get("observe").getAsString()));
                }
                out.println(new Gson().toJson(line));
            }
            var repo = ServerPacksSource.createVanillaTrustedRepository();
            repo.reload();
            repo.setSelected(List.of("vanilla"));
            int index = 0;
            for (JsonElement entry : corpus.getAsJsonArray("packs")) {
                JsonObject one = entry.getAsJsonObject();
                Path root = scratch.resolve("pack" + index++);
                Files.createDirectories(root);
                Files.writeString(root.resolve("pack.mcmeta"), "{\"pack\":{\"description\":\"t\",\"min_format\":[102,0],\"max_format\":[102,0]}}");
                for (Map.Entry<String, JsonElement> file : one.getAsJsonObject("files").entrySet()) {
                    Path path = root.resolve(file.getKey());
                    Files.createDirectories(path.getParent());
                    Files.writeString(path, new Gson().toJson(file.getValue()));
                }
                if (one.has("tag_chain")) {
                    int length = one.get("tag_chain").getAsInt();
                    Path tags = root.resolve("data/test/tags/dialog/chain");
                    Files.createDirectories(tags);
                    for (int i = 0; i < length; i++) {
                        String value = i + 1 == length ? "test:leaf" : String.format("#test:chain/%05d", i + 1);
                        Files.writeString(tags.resolve(String.format("%05d.json", i)), "{\"values\":[\"" + value + "\"]}");
                    }
                }
                var packs = new ArrayList<>(repo.openAllSelected());
                packs.add(new PathPackResources(new PackLocationInfo("test", Component.literal("t"), PackSource.WORLD, Optional.empty()), root));
                var resources = new MultiPackResourceManager(PackType.SERVER_DATA, packs);
                var layers = RegistryLayer.createRegistryAccess();
                var staticTags = TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(RegistryLayer.STATIC));
                var context = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), staticTags);
                JsonObject line = new JsonObject();
                line.addProperty("name", one.get("name").getAsString());
                try {
                    var registries = RegistryDataLoader.load(resources, context, RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
                    line.addProperty("accept", true);
                    if (one.has("observe_tags")) {
                        var dialogs = registries.lookupOrThrow(Registries.DIALOG);
                        var tags = new JsonObject();
                        for (var name : one.getAsJsonArray("observe_tags")) {
                            var tag = TagKey.create(Registries.DIALOG, Identifier.parse(name.getAsString()));
                            var entries = new JsonArray();
                            dialogs.get(tag).orElseThrow().stream().forEach(holder ->
                                entries.add(holder.unwrapKey().orElseThrow().identifier().toString()));
                            tags.add(name.getAsString(), entries);
                        }
                        line.add("round_trip", tags);
                    }
                } catch (RuntimeException error) {
                    Throwable cause = error.getCause() != null ? error.getCause() : error;
                    if (!(cause instanceof ReportedException reported)) throw error;
                    String report = reported.getReport().getFriendlyReport(net.minecraft.ReportType.CRASH);
                    int at = report.indexOf("Errors:");
                    int end = report.indexOf("Stacktrace", at);
                    line.addProperty("accept", false);
                    line.addProperty("error", report.substring(at + "Errors:".length(), end).trim().replaceAll("\\s*\\n\\s*", "\n"));
                }
                out.println(new Gson().toJson(line));
            }
        }
    }

    private static JsonObject observe(Dialog dialog, String what) {
        JsonObject out = new JsonObject();
        switch (what) {
            case "button" -> {
                var button = ((NoticeDialog) dialog).action();
                var label = button.button().label();
                out.addProperty("label", label.getContents() instanceof TranslatableContents t ? t.getKey() : label.getString());
                out.addProperty("width", button.button().width());
                out.addProperty("has_action", button.action().isPresent());
            }
            case "item" -> {
                var stack = ((ItemBody) dialog.common().body().getFirst()).item();
                out.addProperty("id", stack.item().unwrapKey().orElseThrow().identifier().toString());
                out.addProperty("count", stack.count());
            }
            case "named_holder" -> {
                var click = (ClickEvent.ShowDialog) dialog.common().title().getStyle().getClickEvent();
                out.addProperty("id", click.dialog().unwrapKey().orElseThrow().identifier().toString());
            }
            case "inline_button" -> {
                var click = (ClickEvent.ShowDialog) dialog.common().title().getStyle().getClickEvent();
                return observe(click.dialog().value(), "button");
            }
            case "inline_custom_activation" -> {
                var click = (ClickEvent.ShowDialog) dialog.common().title().getStyle().getClickEvent();
                var button = ((NoticeDialog) click.dialog().value()).action();
                var activated = (ClickEvent.Custom) button.action().orElseThrow().createAction(Map.of()).orElseThrow();
                out.addProperty("width", button.button().width());
                out.addProperty("id", activated.id().toString());
                out.addProperty("payload", activated.payload().orElseThrow().toString());
            }
            case "multiline" -> {
                var options = ((TextInput) dialog.common().inputs().getFirst().control()).multiline().orElseThrow();
                out.addProperty("max_lines", options.maxLines().orElseThrow());
                // Client InputControlHandlers.TextInputHandler.addControl uses the explicit
                // height first, otherwise fitting 9 pixels per line plus 8, up to 512.
                out.addProperty("height", options.height().orElseGet(() -> Math.min(9 * options.maxLines().orElse(4) + 8, 512)));
            }
            default -> throw new IllegalArgumentException(what);
        }
        return out;
    }
}
