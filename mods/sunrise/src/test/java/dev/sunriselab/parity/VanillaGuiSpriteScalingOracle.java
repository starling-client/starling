package dev.sunriselab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.blaze3d.vertex.VertexConsumer;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.renderer.state.gui.TiledBlitRenderState;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.client.resources.metadata.animation.FrameSize;
import net.minecraft.client.resources.metadata.gui.GuiMetadataSection;
import net.minecraft.client.resources.metadata.gui.GuiSpriteScaling;
import net.minecraft.resources.Identifier;
import net.minecraft.server.packs.resources.ResourceMetadata;
import org.joml.Matrix3x2f;

/** Runs the pinned GUI sprite metadata codec and TiledBlitRenderState's vertices, without GPU draws. */
public final class VanillaGuiSpriteScalingOracle {
    private static JsonObject decode(JsonObject row) {
        var out = new JsonObject();
        GuiSpriteScaling scaling;
        try {
            var bytes = row.get("json").getAsString().getBytes(StandardCharsets.UTF_8);
            var metadata = ResourceMetadata.fromJsonStream(new ByteArrayInputStream(bytes));
            scaling = metadata.getSection(GuiMetadataSection.TYPE).orElse(GuiMetadataSection.DEFAULT).scaling();
        } catch (Exception error) {
            out.addProperty("error", true);
            return out;
        }
        switch (scaling) {
            case GuiSpriteScaling.Tile tile -> {
                out.addProperty("type", "tile");
                out.addProperty("width", tile.width());
                out.addProperty("height", tile.height());
            }
            case GuiSpriteScaling.NineSlice nine -> {
                out.addProperty("type", "nine_slice");
                out.addProperty("width", nine.width());
                out.addProperty("height", nine.height());
                var border = new JsonArray();
                var b = nine.border();
                for (int side : new int[] {b.left(), b.top(), b.right(), b.bottom()}) border.add(side);
                out.add("border", border);
                out.addProperty("stretch_inner", nine.stretchInner());
            }
            default -> out.addProperty("type", "stretch");
        }
        return out;
    }

    private static int i(JsonObject row, String key) {
        return row.get(key).getAsInt();
    }

    private static JsonObject tile(JsonObject row) throws Exception {
        var contents = new SpriteContents(Identifier.parse("sunrise:probe"),
            new FrameSize(i(row, "contents_w"), i(row, "contents_h")),
            new NativeImage(i(row, "contents_w"), i(row, "contents_h"), true));
        Constructor<TextureAtlasSprite> constructor = TextureAtlasSprite.class.getDeclaredConstructor(
            Identifier.class, SpriteContents.class, int.class, int.class, int.class, int.class, int.class);
        constructor.setAccessible(true);
        var sprite = constructor.newInstance(Identifier.parse("sunrise:atlas"), contents,
            i(row, "atlas_w"), i(row, "atlas_h"), i(row, "x"), i(row, "y"), 0);
        int spriteWidth = i(row, "sprite_w"), spriteHeight = i(row, "sprite_h");
        int textureX = i(row, "texture_x"), textureY = i(row, "texture_y");
        int tileWidth = i(row, "tile_w"), tileHeight = i(row, "tile_h");
        int x = i(row, "x0"), y = i(row, "y0"), width = i(row, "w"), height = i(row, "h");
        // GuiGraphicsExtractor.blitTiledSprite's arguments to innerTiledBlit.
        var state = new TiledBlitRenderState(null, null, new Matrix3x2f(), tileWidth, tileHeight, x, y, x + width,
            y + height, sprite.getU((float) textureX / spriteWidth), sprite.getU((float) (textureX + tileWidth) / spriteWidth),
            sprite.getV((float) textureY / spriteHeight), sprite.getV((float) (textureY + tileHeight) / spriteHeight),
            -1, null, null);
        var vertices = new ArrayList<float[]>();
        var consumer = (VertexConsumer) Proxy.newProxyInstance(VertexConsumer.class.getClassLoader(),
            new Class<?>[] {VertexConsumer.class}, new InvocationHandler() {
                public Object invoke(Object proxy, Method method, Object[] args) throws Throwable {
                    switch (method.getName()) {
                        case "addVertex" -> {
                            if (args.length == 3 && args[0] instanceof Float) {
                                vertices.add(new float[] {(Float) args[0], (Float) args[1], 0, 0});
                                return proxy;
                            }
                        }
                        case "setUv" -> {
                            var last = vertices.get(vertices.size() - 1);
                            last[2] = (Float) args[0];
                            last[3] = (Float) args[1];
                            return proxy;
                        }
                        case "setColor" -> {
                            return proxy;
                        }
                        default -> {}
                    }
                    if (method.isDefault()) return InvocationHandler.invokeDefault(proxy, method, args);
                    throw new UnsupportedOperationException(method.toString());
                }
            });
        state.buildVertices(consumer);
        var quads = new JsonArray();
        for (int q = 0; q + 3 < vertices.size(); q += 4) {
            // A quad's (x0, y0, u0, v0) corner, then its (x1, y1, u1, v1) corner, as float bits.
            float[] a = vertices.get(q), c = vertices.get(q + 2);
            var quad = new JsonArray();
            for (float v : new float[] {a[0], a[1], c[0], c[1], a[2], a[3], c[2], c[3]})
                quad.add(Integer.toUnsignedLong(Float.floatToRawIntBits(v)));
            quads.add(quad);
        }
        var out = new JsonObject();
        out.add("quads", quads);
        return out;
    }

    public static void main(String[] args) throws Exception {
        var gson = new GsonBuilder().disableHtmlEscaping().create();
        try (var out = Files.newBufferedWriter(Path.of(args[1]))) {
            for (var line : Files.readAllLines(Path.of(args[0]))) {
                var row = JsonParser.parseString(line).getAsJsonObject();
                var result = new JsonObject();
                result.add("name", row.get("name"));
                result.add("value", row.get("kind").getAsString().equals("decode") ? decode(row) : tile(row));
                out.write(gson.toJson(result));
                out.write("\n");
            }
        }
    }
}
