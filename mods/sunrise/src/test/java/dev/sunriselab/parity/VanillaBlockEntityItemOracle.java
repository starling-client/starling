package dev.sunriselab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.blaze3d.vertex.PoseStack;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.CompletableFuture;
import java.util.stream.Stream;
import net.minecraft.client.model.geom.EntityModelSet;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.blockentity.*;
import net.minecraft.client.renderer.blockentity.state.*;
import net.minecraft.client.renderer.item.*;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.client.renderer.texture.MissingTextureAtlasSprite;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.client.renderer.texture.SpriteLoader;
import net.minecraft.client.resources.model.*;
import net.minecraft.client.resources.model.cuboid.*;
import net.minecraft.client.resources.model.sprite.SpriteGetter;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryOps;
import net.minecraft.world.entity.ItemOwner;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GrassColor;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.entity.*;
import net.minecraft.world.level.block.entity.vault.VaultBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

/**
 * Original oracle for the items block entity renderers draw: campfires, suspicious sand and
 * gravel, shelves and vaults. Each case builds the vanilla block entity in a level holding its
 * block and the case's light, runs the real renderer's extractRenderState with a real
 * ItemModelResolver (VanillaItemModelOracle's baked models), then its submit, under the pose
 * LevelRenderer gives it (translated from the camera). Every ItemStackRenderState.submit is
 * recorded in order: the stack and the display context and seed it was resolved with, the
 * pose's matrix as raw float bits, and the light.
 *
 * Arguments: extracted client resources, item defaults (items.json), cases (JSONL), output (JSONL).
 */
public final class VanillaBlockEntityItemOracle {
    /** What ItemModelResolver.updateForTopItem was asked for one render state. */
    record Resolved(ItemStack stack, ItemDisplayContext context, int seed) {}

    static final Map<ItemStackRenderState, Resolved> RESOLVED = new IdentityHashMap<>();
    static JsonArray submits;

    /** The baked models of VanillaItemModelOracle, noting each top-level resolution. */
    static final class Recording extends ItemModelResolver {
        final ItemModelResolver models;
        Recording(ItemModelResolver models) { super(null); this.models = models; }
        @Override public void updateForTopItem(ItemStackRenderState output, ItemStack item, ItemDisplayContext context, Level level, ItemOwner owner, int seed) {
            RESOLVED.put(output, new Resolved(item.copy(), context, seed));
            super.updateForTopItem(output, item, context, level, owner, seed);
        }
        @Override public void appendItemLayers(ItemStackRenderState output, ItemStack item, ItemDisplayContext context, Level level, ItemOwner owner, int seed) {
            models.appendItemLayers(output, item, context, level, owner, seed);
        }
    }

    /** Stands in for a renderer's resolved state: answers as it does and records each submit. */
    static final class Rec extends ItemStackRenderState {
        final ItemStackRenderState inner;
        Rec(ItemStackRenderState inner) { this.inner = inner; }
        @Override public boolean isEmpty() { return inner.isEmpty(); }
        @Override public AABB getModelBoundingBox() { return inner.getModelBoundingBox(); }
        @Override public void submit(PoseStack pose, SubmitNodeCollector collector, int light, int overlay, int outline) {
            var r = RESOLVED.get(inner);
            var row = new JsonObject();
            row.addProperty("item", BuiltInRegistries.ITEM.getKey(r.stack().getItem()).toString());
            row.addProperty("count", r.stack().getCount());
            row.addProperty("context", r.context().getSerializedName());
            row.addProperty("seed", r.seed());
            var m = new float[16];
            pose.last().pose().get(m);
            var matrix = new JsonArray();
            for (float v : m) matrix.add(Integer.toHexString(Float.floatToRawIntBits(v)));
            row.add("matrix", matrix);
            row.addProperty("light", light);
            submits.add(row);
        }
    }

    static Rec rec(ItemStackRenderState state) { return state == null ? null : new Rec(state); }

    static void set(Object target, Class<?> type, String name, Object value) throws Exception {
        var f = VanillaBlockEntityRenderOracle.field(type, name);
        f.set(target, value);
    }

    static ItemStack stack(JsonArray item) {
        return new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(item.get(0).getAsString())), item.get(1).getAsInt());
    }

    /** VanillaItemModelOracle.main's bootstrap and bake, without its cases. */
    static ItemModelResolver bake(Path root, Path defaultsPath) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var repo = net.minecraft.server.packs.repository.ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(List.of("vanilla"));
        try (var data = new net.minecraft.server.packs.resources.MultiPackResourceManager(net.minecraft.server.packs.PackType.SERVER_DATA, repo.openAllSelected())) {
            var layers = net.minecraft.server.RegistryLayer.createRegistryAccess();
            net.minecraft.tags.TagLoader.loadTagsForExistingRegistries(data, layers.getLayer(net.minecraft.server.RegistryLayer.STATIC))
                .forEach(net.minecraft.core.Registry.PendingTags::apply);
        }
        ItemModels.bootstrap();
        net.minecraft.client.renderer.special.SpecialModelRenderers.bootstrap();
        net.minecraft.client.color.item.ItemTintSources.bootstrap();
        net.minecraft.client.renderer.item.properties.select.SelectItemModelProperties.bootstrap();
        net.minecraft.client.renderer.item.properties.conditional.ConditionalItemModelProperties.bootstrap();
        net.minecraft.client.renderer.item.properties.numeric.RangeSelectItemModelProperties.bootstrap();
        net.minecraft.client.renderer.texture.atlas.SpriteSources.bootstrap();
        var u = Class.forName("sun.misc.Unsafe");
        var uf = u.getDeclaredField("theUnsafe"); uf.setAccessible(true);
        VanillaItemModelOracle.unsafe = uf.get(null);
        VanillaItemModelOracle.allocate = u.getMethod("allocateInstance", Class.class);
        VanillaItemModelOracle.root = root;
        VanillaItemModelOracle.registries = VanillaRegistries.createWorldLookup();
        VanillaItemModelOracle.ops = RegistryOps.create(JsonOps(), VanillaItemModelOracle.registries);
        var defaults = JsonParser.parseString(Files.readString(defaultsPath)).getAsJsonObject();
        for (var item : BuiltInRegistries.ITEM) {
            var json = defaults.get(BuiltInRegistries.ITEM.getKey(item).toString());
            var map = json == null ? net.minecraft.core.component.DataComponentMap.EMPTY
                : net.minecraft.core.component.DataComponentMap.CODEC.parse(VanillaItemModelOracle.ops, json).getOrThrow();
            item.builtInRegistryHolder().bindComponents(map);
        }
        try (var in = Files.newInputStream(root.resolve("assets/minecraft/textures/colormap/grass.png")); var image = NativeImage.read(in)) {
            GrassColor.init(image.getPixels());
        }
        var models = new HashMap<Identifier, UnbakedModel>();
        try (Stream<Path> files = Files.walk(root.resolve("assets/minecraft/models"))) {
            for (var file : (Iterable<Path>) files.filter(p -> p.toString().endsWith(".json"))::iterator) {
                var relative = root.resolve("assets/minecraft/models").relativize(file).toString().replace('\\', '/');
                try (var reader = Files.newBufferedReader(file)) {
                    models.put(Identifier.withDefaultNamespace(relative.substring(0, relative.length() - 5)), CuboidModel.fromStream(reader));
                } catch (RuntimeException ignored) {}
            }
        }
        var items = new TreeMap<Identifier, ClientItem>();
        try (Stream<Path> files = Files.list(root.resolve("assets/minecraft/items"))) {
            for (var file : (Iterable<Path>) files.sorted()::iterator) {
                var name = file.getFileName().toString();
                VanillaItemModelOracle.decode(Identifier.withDefaultNamespace(name.substring(0, name.length() - 5)), Files.readString(file), items);
            }
        }
        var missingModel = MissingCuboidModel.missingModel();
        var discovery = new ModelDiscovery(models, missingModel);
        discovery.addSpecialModel(ItemModelGenerator.GENERATED_ITEM_MODEL_ID, new ItemModelGenerator());
        items.values().forEach(info -> discovery.addRoot(info.model()));
        var resolved = discovery.resolve();
        var missingSprite = new VanillaItemModelOracle.Sprite(VanillaItemModelOracle.BLOCKS, MissingTextureAtlasSprite.create());
        var blockAtlas = new SpriteLoader.Preparations(16, 16, 0, missingSprite, new VanillaItemModelOracle.Regions(false), CompletableFuture.completedFuture(null));
        var itemAtlas = new SpriteLoader.Preparations(16, 16, 0, missingSprite, new VanillaItemModelOracle.Regions(true), CompletableFuture.completedFuture(null));
        var materials = new net.minecraft.client.resources.model.sprite.MaterialBaker(blockAtlas, itemAtlas);
        var dummy = new VanillaItemModelOracle.Sprite(VanillaItemModelOracle.ITEMS, new SpriteContents(Identifier.parse("minecraft:dummy"), new net.minecraft.client.resources.metadata.animation.FrameSize(16, 16), new NativeImage(16, 16, true)));
        SpriteGetter sprites = id -> dummy;
        var bakery = new ModelBakery(EntityModelSet.vanilla(), sprites, null, Map.of(), VanillaItemModelOracle.tagged(items), resolved, discovery.missingModel());
        var baked = bakery.bakeModels(materials, Runnable::run).join();
        var resolver = new VanillaItemModelOracle.Resolver();
        resolver.models = baked.itemStackModels(); resolver.properties = baked.itemProperties(); resolver.missing = baked.missingModels().item();
        return resolver;
    }

    static com.mojang.serialization.DynamicOps<JsonElement> JsonOps() { return com.mojang.serialization.JsonOps.INSTANCE; }

    static JsonArray run(JsonObject c, Map<String, BlockEntityRenderer> renderers) throws Exception {
        BlockState state = VanillaBlockEntityRenderOracle.parseState(c.get("block").getAsString());
        var p = c.getAsJsonArray("pos");
        var pos = new BlockPos(p.get(0).getAsInt(), p.get(1).getAsInt(), p.get(2).getAsInt());
        var be = ((EntityBlock) state.getBlock()).newBlockEntity(pos, state);
        var level = VanillaItemModelOracle.make(VanillaBlockEntityRenderOracle.CaseLevel.class);
        level.blocks = Map.of(pos, state);
        level.minY = -64;
        level.height = 384;
        var light = c.getAsJsonArray("light");
        level.sky = light.get(0).getAsInt();
        level.block = light.get(1).getAsInt();
        be.setLevel(level);
        float partial = 0.0F;
        switch (c.get("kind").getAsString()) {
            case "campfire", "shelf" -> {
                var items = c.get("kind").getAsString().equals("campfire") ? ((CampfireBlockEntity) be).getItems() : ((ShelfBlockEntity) be).getItems();
                for (var e : c.getAsJsonArray("items")) {
                    var slot = e.getAsJsonArray();
                    var list = new JsonArray(); list.add(slot.get(1)); list.add(slot.get(2));
                    items.set(slot.get(0).getAsInt(), stack(list));
                }
                if (be instanceof ShelfBlockEntity) set(be, ShelfBlockEntity.class, "alignItemsToBottom", c.get("align").getAsBoolean());
            }
            case "brushable" -> {
                set(be, BrushableBlockEntity.class, "item", stack(c.getAsJsonArray("item")));
                set(be, BrushableBlockEntity.class, "hitDirection", Direction.from3DDataValue(c.get("hit").getAsInt()));
            }
            case "vault" -> {
                var vault = (VaultBlockEntity) be;
                set(vault.getSharedData(), vault.getSharedData().getClass(), "displayItem", c.has("item") ? stack(c.getAsJsonArray("item")) : ItemStack.EMPTY);
                var spin = c.getAsJsonArray("spin");
                set(vault.getClientData(), vault.getClientData().getClass(), "previousSpin", spin.get(0).getAsFloat());
                set(vault.getClientData(), vault.getClientData().getClass(), "currentSpin", spin.get(1).getAsFloat());
                partial = spin.get(2).getAsFloat();
            }
            default -> throw new IllegalArgumentException(c.get("kind").getAsString());
        }
        var renderer = renderers.get(c.get("kind").getAsString());
        var cam = c.getAsJsonArray("camera");
        var cameraPos = new Vec3(cam.get(0).getAsDouble(), cam.get(1).getAsDouble(), cam.get(2).getAsDouble());
        if (!renderer.shouldRender(be, cameraPos)) return new JsonArray();
        var rs = (BlockEntityRenderState) renderer.createRenderState();
        renderer.extractRenderState(be, rs, partial, cameraPos, null);
        // Each resolved state is swapped for one that records its submits.
        if (rs instanceof CampfireRenderState campfire) campfire.items.replaceAll(VanillaBlockEntityItemOracle::rec);
        if (rs instanceof BrushableBlockRenderState brushable) set(brushable, BrushableBlockRenderState.class, "itemState", rec(brushable.itemState));
        if (rs instanceof ShelfRenderState shelf) for (int i = 0; i < shelf.items.length; i++) shelf.items[i] = rec(shelf.items[i]);
        if (rs instanceof VaultRenderState vault && vault.displayItem != null)
            set(vault.displayItem, net.minecraft.client.renderer.entity.state.ItemClusterRenderState.class, "item", rec(vault.displayItem.item));
        submits = new JsonArray();
        // LevelRenderer.submitBlockEntities.
        var pose = new PoseStack();
        pose.translate(pos.getX() - cameraPos.x, pos.getY() - cameraPos.y, pos.getZ() - cameraPos.z);
        var camera = new CameraRenderState();
        camera.pos = cameraPos;
        renderer.submit(rs, pose, ignoring(), camera);
        return submits;
    }

    /** No renderer here submits anything but its items, which Rec records. */
    static SubmitNodeCollector ignoring() {
        InvocationHandler handler = (proxy, method, args) -> {
            if (method.getName().equals("order")) return proxy;
            throw new IllegalStateException("unexpected " + method.getName());
        };
        return (SubmitNodeCollector) Proxy.newProxyInstance(SubmitNodeCollector.class.getClassLoader(), new Class<?>[] {SubmitNodeCollector.class}, handler);
    }

    public static void main(String[] args) throws Exception {
        var resolver = new Recording(bake(Path.of(args[0]), Path.of(args[1])));
        var context = new BlockEntityRendererProvider.Context(null, null, resolver, null, null, null, null, null);
        Map<String, BlockEntityRenderer> renderers = Map.of(
            "campfire", new CampfireRenderer(context),
            "brushable", new BrushableBlockRenderer(context),
            "shelf", new ShelfRenderer(context),
            "vault", new VaultRenderer(context));
        var gson = new Gson();
        var out = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[2]))) {
            if (line.isBlank()) continue;
            var c = JsonParser.parseString(line).getAsJsonObject();
            var result = new JsonObject();
            result.addProperty("name", c.get("name").getAsString());
            try {
                RESOLVED.clear();
                result.add("submits", run(c, renderers));
            } catch (Throwable error) {
                var cause = error instanceof InvocationTargetException && error.getCause() != null ? error.getCause() : error;
                var sw = new java.io.StringWriter();
                cause.printStackTrace(new java.io.PrintWriter(sw));
                result.addProperty("error", sw.toString().lines().limit(8).reduce("", (a, b) -> a + b + " | "));
            }
            out.append(gson.toJson(result)).append('\n');
        }
        Files.writeString(Path.of(args[3]), out.toString(), StandardCharsets.UTF_8);
    }
}
