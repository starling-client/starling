package dev.sunriselab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.QuadInstance;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.renderpearl.api.pipeline.BlendFunction;
import com.mojang.renderpearl.api.pipeline.ColorTargetState;
import com.mojang.renderpearl.api.pipeline.DepthStencilState;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import com.mojang.serialization.JsonOps;
import java.io.InputStreamReader;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.*;
import java.util.concurrent.CompletableFuture;
import net.minecraft.SharedConstants;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.TextureFilteringMethod;
import net.minecraft.client.gui.render.DynamicAtlasAllocator;
import net.minecraft.client.model.Model;
import net.minecraft.client.model.geom.EntityModelSet;
import net.minecraft.client.model.geom.builders.UVPair;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.OrderedSubmitNodeCollector;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.ShaderDefines;
import net.minecraft.client.renderer.Sheets;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.block.MovingBlockRenderState;
import net.minecraft.client.renderer.block.dispatch.BlockStateModelPart;
import net.minecraft.client.renderer.entity.DisplayRenderer;
import net.minecraft.client.renderer.entity.EntityRenderDispatcher;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.client.renderer.entity.state.ItemDisplayEntityRenderState;
import net.minecraft.client.renderer.feature.ItemFeatureRenderer;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.gizmos.DrawableGizmoPrimitives;
import net.minecraft.client.renderer.item.ClientItem;
import net.minecraft.client.renderer.item.ItemModel;
import net.minecraft.client.renderer.item.ItemModelResolver;
import net.minecraft.client.renderer.item.ItemModels;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.client.renderer.oit.OitPipelineSet;
import net.minecraft.client.renderer.rendertype.RenderSetup;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.rendertype.RenderTypes;
import net.minecraft.client.renderer.rendertype.TextureTransform;
import net.minecraft.client.renderer.special.SpecialModelRenderers;
import net.minecraft.client.renderer.state.GameRenderState;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.client.renderer.state.level.QuadParticleRenderState;
import net.minecraft.client.renderer.texture.SpriteLoader;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.client.renderer.texture.UvMapping;
import net.minecraft.client.renderer.texture.atlas.SpriteSources;
import net.minecraft.client.resources.model.ModelBakery;
import net.minecraft.client.resources.model.ModelDiscovery;
import net.minecraft.client.resources.model.UnbakedModel;
import net.minecraft.client.resources.model.cuboid.CuboidModel;
import net.minecraft.client.resources.model.cuboid.ItemModelGenerator;
import net.minecraft.client.resources.model.cuboid.MissingCuboidModel;
import net.minecraft.client.resources.model.geometry.BakedQuad;
import net.minecraft.client.resources.model.geometry.ItemQuads;
import net.minecraft.client.resources.model.sprite.Material;
import net.minecraft.client.resources.model.sprite.MaterialBaker;
import net.minecraft.client.resources.model.sprite.SpriteGetter;
import net.minecraft.client.resources.model.sprite.SpriteId;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.data.AtlasIds;
import net.minecraft.network.chat.Component;
import net.minecraft.network.syncher.EntityDataAccessor;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryOps;
import net.minecraft.server.packs.PackLocationInfo;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.PathPackResources;
import net.minecraft.server.packs.repository.PackSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.server.packs.resources.Resource;
import net.minecraft.server.packs.resources.ResourceManager;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.util.StrictJsonParser;
import net.minecraft.util.Util;
import net.minecraft.world.entity.Display;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.ItemOwner;
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BannerPattern;
import net.minecraft.world.level.block.entity.BannerPatternLayers;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.VoxelShape;
import org.joml.Matrix4f;
import org.joml.Quaternionf;
import org.joml.Vector3f;
import org.joml.Vector4f;

/**
 * Headless 26.3 oracle for enchantment glint: TextureTransform matrices,
 * SpriteLoader atlases (items, blocks, shield_patterns), ItemDisplayRenderer +
 * ItemFeatureRenderer (end_crystal EID 83245 and all ItemDisplayContexts),
 * block-atlas items (stone), special decal foil (compass/clock), and
 * entity-glint special models (trident, plain shield, patterned shield).
 */
public final class VanillaItemGlintOracle {
    private static final Gson JSON = new GsonBuilder().serializeNulls().create();

    static Object unsafe;
    static Method allocate;
    static Method objectFieldOffset;
    static Method putObject;

    static long frozenNanos = 0L;

    @SuppressWarnings("unchecked")
    static <T> T alloc(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static void putField(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        if (target == null) {
            f.setAccessible(true);
            f.set(null, value);
        } else if (f.getType() == boolean.class) {
            long offset = (long) objectFieldOffset.invoke(unsafe, f);
            unsafe.getClass().getMethod("putBoolean", Object.class, long.class, boolean.class)
                .invoke(unsafe, target, offset, (Boolean) value);
        } else {
            long offset = (long) objectFieldOffset.invoke(unsafe, f);
            putObject.invoke(unsafe, target, offset, value);
        }
    }

    static Object getField(Object target, Class<?> owner, String name) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        return f.get(target);
    }

    static String hex32(float v) {
        return String.format(Locale.ROOT, "0x%08x", Float.floatToRawIntBits(v));
    }

    static long u32(float v) {
        return Integer.toUnsignedLong(Float.floatToRawIntBits(v));
    }

    static long[] matrixColsBits(Matrix4f m) {
        float[] a = new float[16];
        m.get(a);
        long[] out = new long[16];
        for (int i = 0; i < 16; i++) {
            out[i] = u32(a[i]);
        }
        return out;
    }

    static long[] transformUvBits(Matrix4f textureMat, float u, float v) {
        Vector4f out = textureMat.transform(new Vector4f(u, v, 0.0F, 1.0F));
        return new long[] {u32(out.x), u32(out.y)};
    }

    static Map<String, Object> map(Object... kv) {
        Map<String, Object> m = new LinkedHashMap<>();
        for (int i = 0; i < kv.length; i += 2) {
            m.put((String) kv[i], kv[i + 1]);
        }
        return m;
    }

    static final class StubLevel extends net.minecraft.server.level.ServerLevel {
        private static final net.minecraft.server.ServerScoreboard SCOREBOARD = new net.minecraft.server.ServerScoreboard(null);

        StubLevel() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }

        @Override
        public int getNextEntityId() {
            return 83245;
        }

        @Override
        public net.minecraft.core.RegistryAccess registryAccess() {
            return ParityData.access;
        }

        @Override
        public net.minecraft.server.ServerScoreboard getScoreboard() {
            return SCOREBOARD;
        }
    }

    static final class HeadlessItemDisplay extends Display.ItemDisplay {
        HeadlessItemDisplay(EntityType<?> type, Level level) {
            super(type, level);
        }
    }

    static final class Resolver extends ItemModelResolver {
        Map<Identifier, ItemModel> models;
        Map<Identifier, ClientItem.Properties> properties;
        ItemModel missing;

        Resolver() {
            super(null);
        }

        @Override
        public void appendItemLayers(
            ItemStackRenderState output,
            ItemStack item,
            ItemDisplayContext context,
            Level level,
            ItemOwner owner,
            int seed
        ) {
            Identifier id = item.get(DataComponents.ITEM_MODEL);
            if (id != null) {
                output.setOversizedInGui(
                    properties.getOrDefault(id, ClientItem.Properties.DEFAULT).oversizedInGui()
                );
                models
                    .getOrDefault(id, missing)
                    .update(output, item, this, context, level instanceof ClientLevel c ? c : null, owner, seed);
            }
        }
    }

    static final class RecordedVertex {
        float x, y, z;
        float u, v;
        float u3, v3;
        boolean hasUv3;
        float nx, ny, nz;
    }

    static final class RecordingConsumer implements VertexConsumer {
        final List<RecordedVertex> vertices = new ArrayList<>();
        RecordedVertex cur;

        @Override
        public VertexConsumer addVertex(float x, float y, float z) {
            cur = new RecordedVertex();
            cur.x = x;
            cur.y = y;
            cur.z = z;
            vertices.add(cur);
            return this;
        }

        @Override
        public VertexConsumer setColor(int r, int g, int b, int a) {
            return this;
        }

        @Override
        public VertexConsumer setColor(int color) {
            return this;
        }

        @Override
        public VertexConsumer setUv(float u, float v) {
            cur.u = u;
            cur.v = v;
            return this;
        }

        @Override
        public VertexConsumer setUv1(int u, int v) {
            return this;
        }

        @Override
        public VertexConsumer setUv2(int u, int v) {
            return this;
        }

        @Override
        public VertexConsumer setUv3(float u, float v) {
            cur.u3 = u;
            cur.v3 = v;
            cur.hasUv3 = true;
            return this;
        }

        @Override
        public VertexConsumer setNormal(float x, float y, float z) {
            cur.nx = x;
            cur.ny = y;
            cur.nz = z;
            return this;
        }

        @Override
        public VertexConsumer setLineWidth(float width) {
            return this;
        }
    }

    record CapturedItemSubmit(
        PoseStack.Pose pose,
        ItemDisplayContext displayContext,
        int lightCoords,
        int overlayCoords,
        int outlineColor,
        int[] tintLayers,
        ItemQuads quads,
        ItemStackRenderState.FoilType foilType
    ) {}

    record CapturedModelSubmit(
        int order,
        Model<?> model,
        Object state,
        PoseStack.Pose pose,
        RenderType renderType,
        int lightCoords,
        int overlayCoords,
        int tintedColor,
        UvMapping uvMapping,
        int outlineColor
    ) {}

    static final class RecordingCollector implements SubmitNodeCollector {
        final List<CapturedItemSubmit> items = new ArrayList<>();
        final List<CapturedModelSubmit> models = new ArrayList<>();
        final int currentOrder;
        final RecordingCollector root;

        RecordingCollector() {
            this.currentOrder = 0;
            this.root = this;
        }

        RecordingCollector(RecordingCollector root, int order) {
            this.root = root;
            this.currentOrder = order;
        }

        @Override
        public OrderedSubmitNodeCollector order(int order) {
            return new RecordingCollector(this.root, order);
        }

        @Override
        public void submitShadow(PoseStack poseStack, float radius, List<EntityRenderState.ShadowPiece> pieces) {}

        @Override
        public void submitNameTag(
            PoseStack poseStack,
            Vec3 nameTagAttachment,
            int offset,
            Component name,
            boolean seeThrough,
            int lightCoords,
            CameraRenderState camera
        ) {}

        @Override
        public void submitText(
            PoseStack poseStack,
            float x,
            float y,
            FormattedCharSequence string,
            boolean dropShadow,
            net.minecraft.client.gui.Font.DisplayMode displayMode,
            int lightCoords,
            int color,
            int backgroundColor,
            int outlineColor
        ) {}

        @Override
        public void submitTextBackground(
            PoseStack poseStack,
            float x0,
            float y0,
            float x1,
            float y1,
            int color,
            net.minecraft.client.gui.Font.DisplayMode displayMode,
            int lightCoords
        ) {}

        @Override
        public void submitFlame(PoseStack poseStack, EntityRenderState renderState, Quaternionf rotation) {}

        @Override
        public void submitLeash(PoseStack poseStack, EntityRenderState.LeashState leashState) {}

        @Override
        public <S> void submitModel(
            Model<? super S> model,
            S state,
            PoseStack poseStack,
            RenderType renderType,
            int lightCoords,
            int overlayCoords,
            int tintedColor,
            UvMapping uvMapping,
            int outlineColor
        ) {
            root.models.add(
                new CapturedModelSubmit(
                    currentOrder,
                    model,
                    state,
                    poseStack.last().copy(),
                    renderType,
                    lightCoords,
                    overlayCoords,
                    tintedColor,
                    uvMapping,
                    outlineColor
                )
            );
        }

        @Override
        public <S> void submitCrumblingOverlay(
            Model<? super S> model,
            S state,
            PoseStack poseStack,
            RenderType renderType,
            int lightCoords,
            int overlayCoords,
            int tintedColor,
            ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
        ) {}

        @Override
        public void submitMovingBlock(PoseStack poseStack, MovingBlockRenderState movingBlockRenderState, int outlineColor) {}

        @Override
        public void submitBlockModel(
            PoseStack poseStack,
            RenderType renderType,
            List<BlockStateModelPart> parts,
            int[] tintLayers,
            int lightCoords,
            int overlayCoords,
            int outlineColor
        ) {}

        @Override
        public void submitBreakingBlockModel(
            PoseStack poseStack,
            List<BlockStateModelPart> parts,
            int progress,
            boolean isBlockTranslucent
        ) {}

        @Override
        public void submitShapeOutline(
            PoseStack poseStack,
            VoxelShape shape,
            RenderType renderType,
            int color,
            float width,
            boolean afterTerrain
        ) {}

        @Override
        public void submitItem(
            PoseStack poseStack,
            ItemDisplayContext displayContext,
            int lightCoords,
            int overlayCoords,
            int outlineColor,
            int[] tintLayers,
            ItemQuads quads,
            ItemStackRenderState.FoilType foilType
        ) {
            root.items.add(
                new CapturedItemSubmit(
                    poseStack.last().copy(),
                    displayContext,
                    lightCoords,
                    overlayCoords,
                    outlineColor,
                    tintLayers,
                    quads,
                    foilType
                )
            );
        }

        @Override
        public void submitCustomGeometry(
            PoseStack poseStack,
            RenderType renderType,
            SubmitNodeCollector.CustomGeometryRenderer customGeometryRenderer
        ) {}

        @Override
        public void submitQuadParticleGroup(QuadParticleRenderState particles) {}

        @Override
        public void submitGizmoPrimitives(DrawableGizmoPrimitives.Group group, CameraRenderState camera, boolean onTop) {}
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    static void setEntityData(Display display, Class<?> owner, String fieldName, Object value) throws Exception {
        Field f = owner.getDeclaredField(fieldName);
        f.setAccessible(true);
        display.getEntityData().set((EntityDataAccessor) f.get(null), value);
    }

    static RenderSetup renderSetupOf(RenderType rt) throws Exception {
        return (RenderSetup) getField(rt, RenderType.class, "state");
    }

    static String renderTypeName(RenderType rt) throws Exception {
        return (String) getField(rt, RenderType.class, "name");
    }

    static TextureTransform textureTransformOf(RenderType rt) throws Exception {
        return (TextureTransform) getField(renderSetupOf(rt), RenderSetup.class, "textureTransform");
    }

    static Map<String, Object> describePipeline(RenderPipeline pipeline) {
        DepthStencilState ds = pipeline.getDepthStencilState();
        ColorTargetState ct = pipeline.getColorTargetStates().isEmpty() ? null : pipeline.getColorTargetStates().get(0);
        Map<String, Object> blend = null;
        if (ct != null && ct.blendFunction().isPresent()) {
            BlendFunction bf = ct.blendFunction().get();
            blend = map(
                "src_color", bf.color().sourceFactor().name(),
                "dst_color", bf.color().destFactor().name(),
                "color_op", bf.color().op().name(),
                "src_alpha", bf.alpha().sourceFactor().name(),
                "dst_alpha", bf.alpha().destFactor().name(),
                "alpha_op", bf.alpha().op().name()
            );
        }
        ShaderDefines defs = pipeline.getShaderDefines();
        return map(
            "location", pipeline.getLocation().toString(),
            "cull", pipeline.isCull(),
            "depth_test", ds != null ? ds.depthTest().name() : null,
            "depth_write", ds != null ? ds.writeDepth() : null,
            "blend", blend,
            "alpha_cutout", defs.values().get("ALPHA_CUTOUT"),
            "define_values", new TreeMap<>(defs.values()),
            "define_flags", new ArrayList<>(new TreeSet<>(defs.flags()))
        );
    }

    static Map<String, Object> describeOitSet(OitPipelineSet set) {
        return map(
            "depth_bounds", describePipeline(set.depthBoundsPipeline()),
            "transmittance", describePipeline(set.transmittancePipeline()),
            "accumulate", describePipeline(set.accumulatePipeline())
        );
    }

    @SuppressWarnings("unchecked")
    static Map<String, String> texturesOf(RenderType rt) throws Exception {
        Map<String, Object> bindings = (Map<String, Object>) getField(renderSetupOf(rt), RenderSetup.class, "textures");
        Map<String, String> out = new TreeMap<>();
        Method loc = null;
        for (var entry : bindings.entrySet()) {
            if (loc == null) {
                loc = entry.getValue().getClass().getDeclaredMethod("location");
                loc.setAccessible(true);
            }
            out.put(entry.getKey(), loc.invoke(entry.getValue()).toString());
        }
        return out;
    }

    static void setClock(GameRenderState gameRenderState, long millis, double glintSpeed) {
        setClockNanos(gameRenderState, millis * 1_000_000L, glintSpeed);
    }

    static void setClockNanos(GameRenderState gameRenderState, long nanos, double glintSpeed) {
        frozenNanos = nanos;
        gameRenderState.optionsRenderState.glintSpeed = glintSpeed;
        long expectedMillis = nanos / 1_000_000L;
        if (Util.getMillis() != expectedMillis) {
            throw new IllegalStateException("Util.getMillis()=" + Util.getMillis() + " != " + expectedMillis);
        }
    }

    static Map<String, Object> recordAtlas(
        SpriteLoader.Preparations prep,
        String[] spriteNames
    ) {
        Map<String, Object> sprites = new LinkedHashMap<>();
        for (String name : spriteNames) {
            Identifier id = Identifier.parse(name);
            TextureAtlasSprite sp = prep.getSprite(id);
            if (sp == null) {
                sprites.put(name, null);
                continue;
            }
            sprites.put(
                name,
                map(
                    "x", sp.getX(),
                    "y", sp.getY(),
                    "w", sp.contents().width(),
                    "h", sp.contents().height(),
                    "u0_bits", u32(sp.getU0()),
                    "v0_bits", u32(sp.getV0()),
                    "u1_bits", u32(sp.getU1()),
                    "v1_bits", u32(sp.getV1()),
                    "u0", sp.getU0(),
                    "v0", sp.getV0(),
                    "u1", sp.getU1(),
                    "v1", sp.getV1()
                )
            );
        }
        return map(
            "width", prep.width(),
            "height", prep.height(),
            "mip_level", prep.mipLevel(),
            "sprite_count", prep.regions().size(),
            "sprites", sprites
        );
    }

    static int compareBits2D(long[][] a, long[][] b) {
        int cmp = Integer.compare(a.length, b.length);
        if (cmp != 0) return cmp;
        for (int i = 0; i < a.length; i++) {
            cmp = Integer.compare(a[i].length, b[i].length);
            if (cmp != 0) return cmp;
            for (int j = 0; j < a[i].length; j++) {
                cmp = Long.compare(a[i][j], b[i][j]);
                if (cmp != 0) return cmp;
            }
        }
        return 0;
    }

    @SuppressWarnings("unchecked")
    static int compareQuadGeom(Object ao, Object bo) {
        Map<String, Object> a = (Map<String, Object>) ao;
        Map<String, Object> b = (Map<String, Object>) bo;
        int cmp = compareBits2D((long[][]) a.get("pos_bits"), (long[][]) b.get("pos_bits"));
        if (cmp != 0) return cmp;
        return compareBits2D((long[][]) a.get("uv0_bits"), (long[][]) b.get("uv0_bits"));
    }

    static void canonicaliseSideFaces(List<Object> quadRecords) {
        if (quadRecords.size() <= 2) return;
        List<Object> sides = quadRecords.subList(2, quadRecords.size());
        sides.sort(VanillaItemGlintOracle::compareQuadGeom);
        for (int i = 1; i < sides.size(); i++) {
            if (compareQuadGeom(sides.get(i - 1), sides.get(i)) == 0) {
                throw new IllegalStateException("duplicate side-face geometric key at index " + (i + 2));
            }
        }
    }

    static Map<String, Object> evaluateItemSubmits(
        List<CapturedItemSubmit> submits,
        GameRenderState gameRenderState,
        Method computeFoilDecalPose,
        boolean canonicaliseSides
    ) throws Exception {
        List<Object> quadRecords = new ArrayList<>();
        QuadInstance qi = new QuadInstance();
        String firstRenderType = null;
        String firstTextureTransform = null;
        Map<String, String> firstTextures = null;
        String firstFoilType = null;

        for (CapturedItemSubmit submit : submits) {
            ItemStackRenderState.FoilType foil = submit.foilType();
            if (firstFoilType == null) firstFoilType = foil.name().toLowerCase(Locale.ROOT);
            PoseStack.Pose foilDecalPose = foil == ItemStackRenderState.FoilType.SPECIAL
                ? (PoseStack.Pose) computeFoilDecalPose.invoke(null, submit.displayContext(), submit.pose())
                : null;
            qi.setLightCoords(submit.lightCoords());
            qi.setOverlayCoords(submit.overlayCoords());

            for (BakedQuad quad : submit.quads().all()) {
                BakedQuad.MaterialInfo mat = quad.materialInfo();
                RenderType rt = switch (foil) {
                    case NONE -> mat.itemRenderType();
                    case STANDARD -> mat.itemGlintRenderType();
                    case SPECIAL -> mat.itemGlintSpecialRenderType();
                };
                TextureTransform tt = textureTransformOf(rt);
                String rtName = renderTypeName(rt);
                Map<String, String> textures = texturesOf(rt);
                if (firstRenderType == null) {
                    firstRenderType = rtName;
                    firstTextureTransform = tt.toString();
                    firstTextures = textures;
                }

                setClock(gameRenderState, 0L, 0.5);
                Matrix4f matT0 = tt.createMatrix();
                setClock(gameRenderState, 123456L, 0.5);
                Matrix4f matT123456 = tt.createMatrix();

                RecordingConsumer rc = new RecordingConsumer();
                qi.setColor(-1);
                if (foil == ItemStackRenderState.FoilType.SPECIAL) {
                    rc.putBakedQuadWithGlint(submit.pose(), quad, qi, foilDecalPose);
                } else {
                    rc.putBakedQuad(submit.pose(), quad, qi);
                }

                long[][] posBits = new long[4][3];
                long[][] uv0Bits = new long[4][2];
                long[][] uv3Bits = foil == ItemStackRenderState.FoilType.SPECIAL ? new long[4][2] : null;
                long[][] glintT0Bits = new long[4][2];
                long[][] glintT123456Bits = new long[4][2];
                for (int i = 0; i < 4; i++) {
                    RecordedVertex rv = rc.vertices.get(i);
                    posBits[i] = new long[] {u32(rv.x), u32(rv.y), u32(rv.z)};
                    uv0Bits[i] = new long[] {u32(rv.u), u32(rv.v)};
                    float gu = foil == ItemStackRenderState.FoilType.SPECIAL ? rv.u3 : rv.u;
                    float gv = foil == ItemStackRenderState.FoilType.SPECIAL ? rv.v3 : rv.v;
                    if (uv3Bits != null) {
                        uv3Bits[i] = new long[] {u32(rv.u3), u32(rv.v3)};
                    }
                    glintT0Bits[i] = transformUvBits(matT0, gu, gv);
                    glintT123456Bits[i] = transformUvBits(matT123456, gu, gv);
                }
                Map<String, Object> qm = map(
                    "facing", quad.direction().getName(),
                    "sprite", mat.sprite().contents().name().toString(),
                    "render_type", rtName,
                    "texture_transform", tt.toString(),
                    "pos_bits", posBits,
                    "uv0_bits", uv0Bits,
                    "glint_uv_t0_bits", glintT0Bits,
                    "glint_uv_t123456_bits", glintT123456Bits
                );
                if (uv3Bits != null) {
                    qm.put("uv3_bits", uv3Bits);
                }
                quadRecords.add(qm);
            }
        }
        if (canonicaliseSides) {
            canonicaliseSideFaces(quadRecords);
        }
        return map(
            "foil_type", firstFoilType,
            "render_type", firstRenderType,
            "texture_transform", firstTextureTransform,
            "textures", firstTextures,
            "quad_count", quadRecords.size(),
            "quads", quadRecords
        );
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    static List<Object> evaluateModelSubmits(
        List<CapturedModelSubmit> submits,
        GameRenderState gameRenderState
    ) throws Exception {
        List<Object> out = new ArrayList<>();
        PoseStack ps = new PoseStack();
        for (CapturedModelSubmit submit : submits) {
            ps.last().set(submit.pose());
            TextureTransform tt = textureTransformOf(submit.renderType());
            String rtName = renderTypeName(submit.renderType());
            Map<String, String> textures = texturesOf(submit.renderType());
            boolean isGlint = !tt.equals(TextureTransform.DEFAULT_TEXTURING);

            RecordingConsumer rc = new RecordingConsumer();
            VertexConsumer target = submit.uvMapping() != null ? submit.uvMapping().wrap(rc) : rc;
            Model model = submit.model();
            model.setupAnim(submit.state());
            model.renderToBuffer(ps, target, submit.lightCoords(), submit.overlayCoords(), submit.tintedColor());

            setClock(gameRenderState, 0L, 0.5);
            Matrix4f matT0 = tt.createMatrix();
            setClock(gameRenderState, 123456L, 0.5);
            Matrix4f matT123456 = tt.createMatrix();

            int quadCount = rc.vertices.size() / 4;
            List<Object> quads = new ArrayList<>(quadCount);
            for (int q = 0; q < quadCount; q++) {
                long[][] posBits = new long[4][3];
                long[][] uv0Bits = new long[4][2];
                long[][] glintT0Bits = isGlint ? new long[4][2] : null;
                long[][] glintT123456Bits = isGlint ? new long[4][2] : null;
                for (int i = 0; i < 4; i++) {
                    RecordedVertex rv = rc.vertices.get(q * 4 + i);
                    posBits[i] = new long[] {u32(rv.x), u32(rv.y), u32(rv.z)};
                    uv0Bits[i] = new long[] {u32(rv.u), u32(rv.v)};
                    if (isGlint) {
                        glintT0Bits[i] = transformUvBits(matT0, rv.u, rv.v);
                        glintT123456Bits[i] = transformUvBits(matT123456, rv.u, rv.v);
                    }
                }
                Map<String, Object> qm = map("pos_bits", posBits, "uv0_bits", uv0Bits);
                if (isGlint) {
                    qm.put("glint_uv_t0_bits", glintT0Bits);
                    qm.put("glint_uv_t123456_bits", glintT123456Bits);
                }
                quads.add(qm);
            }
            RenderSetup setup = renderSetupOf(submit.renderType());
            RenderPipeline pipeline = (RenderPipeline) getField(setup, RenderSetup.class, "pipeline");
            boolean forceSolidModelPhase = (boolean) getField(setup, RenderSetup.class, "forceSolidModelPhase");
            boolean hasOit = getField(setup, RenderSetup.class, "oitPipelineSet") != null;
            out.add(
                map(
                    "order", submit.order(),
                    "render_type", rtName,
                    "texture_transform", tt.toString(),
                    "textures", textures,
                    "pipeline", describePipeline(pipeline),
                    "force_solid_model_phase", forceSolidModelPhase,
                    "has_oit", hasOit,
                    "has_uv_mapping", submit.uvMapping() != null,
                    "is_glint", isGlint,
                    "quad_count", quadCount,
                    "quads", quads
                )
            );
        }
        return out;
    }

    public static void main(String[] args) throws Exception {
        Path outPath = Path.of(args[0]);
        ParityData.init();
        ItemModels.bootstrap();
        SpecialModelRenderers.bootstrap();
        net.minecraft.client.color.item.ItemTintSources.bootstrap();
        net.minecraft.client.renderer.item.properties.select.SelectItemModelProperties.bootstrap();
        net.minecraft.client.renderer.item.properties.conditional.ConditionalItemModelProperties.bootstrap();
        net.minecraft.client.renderer.item.properties.numeric.RangeSelectItemModelProperties.bootstrap();
        SpriteSources.bootstrap();

        Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
        Field theUnsafe = unsafeClass.getDeclaredField("theUnsafe");
        theUnsafe.setAccessible(true);
        unsafe = theUnsafe.get(null);
        allocate = unsafeClass.getMethod("allocateInstance", Class.class);
        objectFieldOffset = unsafeClass.getMethod("objectFieldOffset", Field.class);
        putObject = unsafeClass.getMethod("putObject", Object.class, long.class, Object.class);

        // Wire headless Minecraft + GameRenderer + GameRenderState + Options.
        Minecraft mc = alloc(Minecraft.class);
        GameRenderer gameRenderer = alloc(GameRenderer.class);
        GameRenderState gameRenderState = new GameRenderState();
        Options options = alloc(Options.class);
        OptionInstance<TextureFilteringMethod> filteringOpt = alloc(OptionInstance.class);
        OptionInstance<Integer> anisoBitOpt = alloc(OptionInstance.class);
        putField(filteringOpt, OptionInstance.class, "value", TextureFilteringMethod.NONE);
        putField(anisoBitOpt, OptionInstance.class, "value", 2);
        putField(options, Options.class, "textureFiltering", filteringOpt);
        putField(options, Options.class, "maxAnisotropyBit", anisoBitOpt);
        putField(gameRenderer, GameRenderer.class, "gameRenderState", gameRenderState);
        putField(mc, Minecraft.class, "gameRenderer", gameRenderer);
        putField(mc, Minecraft.class, "options", options);
        putField(null, Minecraft.class, "instance", mc);
        Util.setTimeSource(() -> frozenNanos);

        ResourceManager resources = ParityFont.resources();

        // 1. Real SpriteLoader atlases from pinned 26.3 client.jar.
        SpriteLoader.Preparations itemsNone = new SpriteLoader(TextureAtlas.LOCATION_ITEMS, 16384)
            .loadAndStitch(resources, AtlasIds.ITEMS, 0, Runnable::run, Set.of())
            .join();
        SpriteLoader.Preparations blocksMip4None = new SpriteLoader(TextureAtlas.LOCATION_BLOCKS, 16384)
            .loadAndStitch(resources, AtlasIds.BLOCKS, 4, Runnable::run, Set.of())
            .join();
        SpriteLoader.Preparations blocksMip0None = new SpriteLoader(TextureAtlas.LOCATION_BLOCKS, 16384)
            .loadAndStitch(resources, AtlasIds.BLOCKS, 0, Runnable::run, Set.of())
            .join();
        SpriteLoader.Preparations shieldPatternsNone = new SpriteLoader(Sheets.SHIELD_SHEET, 16384)
            .loadAndStitch(resources, AtlasIds.SHIELD_PATTERNS, 0, Runnable::run, Set.of())
            .join();

        putField(filteringOpt, OptionInstance.class, "value", TextureFilteringMethod.ANISOTROPIC);
        putField(anisoBitOpt, OptionInstance.class, "value", 2);
        SpriteLoader.Preparations itemsAniso2 = new SpriteLoader(TextureAtlas.LOCATION_ITEMS, 16384)
            .loadAndStitch(resources, AtlasIds.ITEMS, 0, Runnable::run, Set.of())
            .join();
        // Restore NONE filtering for subsequent default checks.
        putField(filteringOpt, OptionInstance.class, "value", TextureFilteringMethod.NONE);

        Map<String, Object> atlasesJson = map(
            "items_none", recordAtlas(itemsNone, new String[] {
                "minecraft:item/end_crystal",
                "minecraft:item/enchanted_book",
                "minecraft:item/compass_16",
                "minecraft:item/compass_00",
                "minecraft:trims/items/boots_trim_quartz",
                "minecraft:missingno"
            }),
            "items_aniso2", recordAtlas(itemsAniso2, new String[] {
                "minecraft:item/end_crystal",
                "minecraft:item/enchanted_book",
                "minecraft:trims/items/boots_trim_quartz",
                "minecraft:missingno"
            }),
            "blocks_mip4_none", recordAtlas(blocksMip4None, new String[] {
                "minecraft:block/stone",
                "minecraft:missingno"
            }),
            "blocks_mip0_none", recordAtlas(blocksMip0None, new String[] {
                "minecraft:block/stone",
                "minecraft:missingno"
            }),
            "shield_patterns_none", recordAtlas(shieldPatternsNone, new String[] {
                "minecraft:entity/shield/shield_base_nopattern",
                "minecraft:entity/shield/shield_base",
                "minecraft:entity/shield/base",
                "minecraft:missingno"
            })
        );

        // 2. Real TextureTransform matrices & transformed sample UVs.
        TextureAtlasSprite endCrystalSprite = itemsNone.getSprite(Identifier.parse("minecraft:item/end_crystal"));
        float[][] sampleUvs = new float[][] {
            {0.0F, 0.0F},
            {1.0F, 1.0F},
            {0.25F, 0.75F},
            {endCrystalSprite.getU0(), endCrystalSprite.getV0()},
            {endCrystalSprite.getU1(), endCrystalSprite.getV1()}
        };
        Object[][] clocks = new Object[][] {
            {0L, 0.5},
            {1_500_000L, 0.5},
            {1_000L * 1_000_000L, 0.7},
            {12345L * 1_000_000L, 0.5},
            {123456L * 1_000_000L, 0.5},
            {123456L * 1_000_000L, 1.0},
            {500000L * 1_000_000L, 0.25},
            {0L, 0.0},
            {999999L * 1_000_000L, 0.5},
            {((1L << 24) + 1L) * 1_000_000L, 0.5},
            {100_000_001L * 1_000_000L, 0.5}
        };
        Object[][] transformSpecs = new Object[][] {
            {"glint_texturing", TextureTransform.GLINT_TEXTURING, 8.0F},
            {"entity_glint_texturing", TextureTransform.ENTITY_GLINT_TEXTURING, 0.5F},
            {"armor_entity_glint_texturing", TextureTransform.ARMOR_ENTITY_GLINT_TEXTURING, 0.16F}
        };
        List<Object> transformsJson = new ArrayList<>();
        for (Object[] spec : transformSpecs) {
            String name = (String) spec[0];
            TextureTransform tt = (TextureTransform) spec[1];
            float scale = (float) spec[2];
            for (Object[] clock : clocks) {
                long nanos = (long) clock[0];
                double speed = (double) clock[1];
                setClockNanos(gameRenderState, nanos, speed);
                long millis = Util.getMillis();
                long ticks = (long) (millis * speed * 8.0);
                float scrollTicks = (float) (ticks % 330_000L);
                Matrix4f mat = tt.createMatrix();
                List<Object> samples = new ArrayList<>();
                for (float[] uv : sampleUvs) {
                    samples.add(
                        map(
                            "in_uv_bits", new long[] {u32(uv[0]), u32(uv[1])},
                            "out_uv_bits", transformUvBits(mat, uv[0], uv[1])
                        )
                    );
                }
                transformsJson.add(
                    map(
                        "name", name,
                        "to_string", tt.toString(),
                        "scale", scale,
                        "scale_bits", u32(scale),
                        "nanos", nanos,
                        "millis", millis,
                        "speed", speed,
                        "speed_f64_bits", String.format(Locale.ROOT, "%016x", Double.doubleToRawLongBits(speed)),
                        "scroll_ticks_bits", u32(scrollTicks),
                        "matrix_cols_bits", matrixColsBits(mat),
                        "samples", samples
                    )
                );
            }
        }

        // 3. Bake real vanilla models from client.jar with real MaterialBaker + real SpriteGetter.
        Map<Identifier, UnbakedModel> unbakedModels = new HashMap<>();
        for (var entry : resources.listResources("models", id -> id.getPath().endsWith(".json")).entrySet()) {
            String path = entry.getKey().getPath();
            String modelPath = path.substring("models/".length(), path.length() - ".json".length());
            Identifier id = Identifier.fromNamespaceAndPath(entry.getKey().getNamespace(), modelPath);
            try (var reader = new InputStreamReader(entry.getValue().open(), StandardCharsets.UTF_8)) {
                unbakedModels.put(id, CuboidModel.fromStream(reader));
            } catch (RuntimeException ignored) {
            }
        }

        var jsonOps = RegistryOps.create(JsonOps.INSTANCE, ParityData.lookup);
        String[] targetItemNames = {"end_crystal", "enchanted_book", "stone", "compass", "clock", "trident", "shield"};
        Map<Identifier, ClientItem> clientItems = new TreeMap<>();
        for (String name : targetItemNames) {
            Identifier fileId = Identifier.withDefaultNamespace("items/" + name + ".json");
            Resource res = resources.getResourceOrThrow(fileId);
            try (var reader = new InputStreamReader(res.open(), StandardCharsets.UTF_8)) {
                ClientItem parsed = ClientItem.CODEC.parse(jsonOps, StrictJsonParser.parse(reader)).getOrThrow();
                clientItems.put(Identifier.withDefaultNamespace(name), parsed);
            }
        }

        ModelDiscovery discovery = new ModelDiscovery(unbakedModels, MissingCuboidModel.missingModel());
        discovery.addSpecialModel(ItemModelGenerator.GENERATED_ITEM_MODEL_ID, new ItemModelGenerator());
        clientItems.values().forEach(ci -> discovery.addRoot(ci.model()));
        var resolvedModels = discovery.resolve();

        MaterialBaker materialBaker = new MaterialBaker(blocksMip4None, itemsNone);
        SpriteGetter spriteGetter = (SpriteId id) -> {
            if (id.atlasLocation().equals(Sheets.SHIELD_SHEET)) {
                TextureAtlasSprite s = shieldPatternsNone.getSprite(id.texture());
                return s != null ? s : shieldPatternsNone.missing();
            }
            if (id.atlasLocation().equals(TextureAtlas.LOCATION_ITEMS)) {
                TextureAtlasSprite s = itemsNone.getSprite(id.texture());
                return s != null ? s : itemsNone.missing();
            }
            TextureAtlasSprite s = blocksMip4None.getSprite(id.texture());
            return s != null ? s : blocksMip4None.missing();
        };

        ModelBakery bakery = new ModelBakery(
            EntityModelSet.vanilla(),
            spriteGetter,
            null,
            Map.of(),
            clientItems,
            resolvedModels,
            discovery.missingModel()
        );
        ModelBakery.BakingResult baked = bakery.bakeModels(materialBaker, Runnable::run).join();

        Resolver resolver = new Resolver();
        resolver.models = baked.itemStackModels();
        resolver.properties = baked.itemProperties();
        resolver.missing = baked.missingModels().item();

        Method computeFoilDecalPose = ItemFeatureRenderer.class.getDeclaredMethod(
            "computeFoilDecalPose",
            ItemDisplayContext.class,
            PoseStack.Pose.class
        );
        computeFoilDecalPose.setAccessible(true);

        // 4. End crystal ItemDisplay EID 83245 driven through REAL Display.ItemDisplay + DisplayRenderer.ItemDisplayRenderer.
        StubLevel stubLevel = alloc(StubLevel.class);
        putField(stubLevel, Level.class, "isClientSide", true);
        EntityType<?> itemDisplayType = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse("minecraft:item_display"));
        HeadlessItemDisplay displayEntity = new HeadlessItemDisplay(itemDisplayType, stubLevel);
        displayEntity.setId(83245);
        setEntityData(displayEntity, Display.class, "DATA_TRANSLATION_ID", new Vector3f(-0.945F, 0.3885F, -4.2F));
        setEntityData(displayEntity, Display.class, "DATA_SCALE_ID", new Vector3f(0.525F, 0.525F, 0.525F));
        setEntityData(displayEntity, Display.class, "DATA_LEFT_ROTATION_ID", new Quaternionf(0.0F, 1.0F, 0.0F, 0.0F));
        setEntityData(displayEntity, Display.class, "DATA_BILLBOARD_RENDER_CONSTRAINTS_ID", (byte) 3);
        setEntityData(displayEntity, Display.class, "DATA_BRIGHTNESS_OVERRIDE_ID", 15728880);
        setEntityData(displayEntity, Display.class, "DATA_VIEW_RANGE_ID", 0.03F);
        setEntityData(displayEntity, Display.ItemDisplay.class, "DATA_ITEM_STACK_ID", new ItemStack(Items.END_CRYSTAL));
        setEntityData(displayEntity, Display.ItemDisplay.class, "DATA_ITEM_DISPLAY_ID", ItemDisplayContext.GUI.getId());
        displayEntity.tick();

        DisplayRenderer.ItemDisplayRenderer displayRenderer = alloc(DisplayRenderer.ItemDisplayRenderer.class);
        EntityRenderDispatcher dispatcher = alloc(EntityRenderDispatcher.class);
        Camera camera = new Camera();
        putField(dispatcher, EntityRenderDispatcher.class, "camera", camera);
        putField(displayRenderer, net.minecraft.client.renderer.entity.EntityRenderer.class, "entityRenderDispatcher", dispatcher);
        putField(displayRenderer, DisplayRenderer.class, "entityRenderDispatcher", dispatcher);
        putField(displayRenderer, DisplayRenderer.ItemDisplayRenderer.class, "itemModelResolver", resolver);
        ItemDisplayEntityRenderState displayState = displayRenderer.createRenderState();
        displayRenderer.extractRenderState(displayEntity, displayState, 1.0F);
        RecordingCollector displayCollector = new RecordingCollector();
        PoseStack displayPose = new PoseStack();
        displayRenderer.submit(displayState, displayPose, displayCollector, new CameraRenderState());

        Map<String, Object> eid83245Json = evaluateItemSubmits(displayCollector.items, gameRenderState, computeFoilDecalPose, true);
        eid83245Json.put("entity_id", 83245);
        eid83245Json.put("item", "minecraft:end_crystal");
        eid83245Json.put("display_context", "gui");
        eid83245Json.put("submit_pose_cols_bits", matrixColsBits(displayCollector.items.get(0).pose().pose()));

        // Verify every ItemDisplayContext matches EID 83245 bit-for-bit, then record canonical quads.
        @SuppressWarnings("unchecked")
        List<Map<String, Object>> eidQuads = (List<Map<String, Object>>) eid83245Json.get("quads");
        Map<String, Map<String, Object>> eidSidesByFacingUv = new HashMap<>();
        for (int i = 2; i < eidQuads.size(); i++) {
            Map<String, Object> q = eidQuads.get(i);
            String k = q.get("facing") + ":" + Arrays.deepToString((long[][]) q.get("uv0_bits"));
            if (eidSidesByFacingUv.put(k, q) != null) {
                throw new IllegalStateException("duplicate eid side key " + k);
            }
        }
        Map<String, Object> contextsJson = new LinkedHashMap<>();
        ItemStack endCrystalStack = new ItemStack(Items.END_CRYSTAL);
        for (ItemDisplayContext ctx : ItemDisplayContext.values()) {
            ItemStackRenderState st = new ItemStackRenderState();
            resolver.updateForTopItem(st, endCrystalStack, ctx, null, null, 0);
            RecordingCollector col = new RecordingCollector();
            st.submit(new PoseStack(), col, 15728880, 0, 0);
            Map<String, Object> eval = evaluateItemSubmits(col.items, gameRenderState, computeFoilDecalPose, true);
            @SuppressWarnings("unchecked")
            List<Map<String, Object>> ctxQuads = (List<Map<String, Object>>) eval.get("quads");
            if (ctxQuads.size() != eidQuads.size()) {
                throw new IllegalStateException("quad count mismatch for " + ctx);
            }
            for (int i = 0; i < 2; i++) {
                if (!Arrays.deepEquals((long[][]) ctxQuads.get(i).get("uv0_bits"), (long[][]) eidQuads.get(i).get("uv0_bits"))
                    || !Arrays.deepEquals((long[][]) ctxQuads.get(i).get("glint_uv_t0_bits"), (long[][]) eidQuads.get(i).get("glint_uv_t0_bits"))
                    || !Arrays.deepEquals((long[][]) ctxQuads.get(i).get("glint_uv_t123456_bits"), (long[][]) eidQuads.get(i).get("glint_uv_t123456_bits"))) {
                    throw new IllegalStateException("glint mismatch for " + ctx + " quad " + i);
                }
            }
            Set<String> seen = new HashSet<>();
            for (int i = 2; i < ctxQuads.size(); i++) {
                Map<String, Object> cq = ctxQuads.get(i);
                String k = cq.get("facing") + ":" + Arrays.deepToString((long[][]) cq.get("uv0_bits"));
                if (!seen.add(k)) {
                    throw new IllegalStateException("duplicate side key for " + ctx + ": " + k);
                }
                Map<String, Object> eq = eidSidesByFacingUv.get(k);
                if (eq == null
                    || !Arrays.deepEquals((long[][]) cq.get("uv0_bits"), (long[][]) eq.get("uv0_bits"))
                    || !Arrays.deepEquals((long[][]) cq.get("glint_uv_t0_bits"), (long[][]) eq.get("glint_uv_t0_bits"))
                    || !Arrays.deepEquals((long[][]) cq.get("glint_uv_t123456_bits"), (long[][]) eq.get("glint_uv_t123456_bits"))) {
                    throw new IllegalStateException("glint mismatch for " + ctx + " side " + k);
                }
            }
            contextsJson.put(
                ctx.getSerializedName(),
                map(
                    "foil_type", eval.get("foil_type"),
                    "render_type", eval.get("render_type"),
                    "texture_transform", eval.get("texture_transform"),
                    "textures", eval.get("textures"),
                    "quad_count", eval.get("quad_count"),
                    "matches_eid_83245_all_quads", true,
                    "quads", ctxQuads
                )
            );
        }

        // 6. Block-textured enchanted item: minecraft:stone with enchantment_glint_override = true.
        ItemStack glintStone = new ItemStack(Items.STONE);
        glintStone.set(DataComponents.ENCHANTMENT_GLINT_OVERRIDE, true);
        Map<String, Object> blockItemJson = new LinkedHashMap<>();
        for (ItemDisplayContext ctx : new ItemDisplayContext[] {ItemDisplayContext.GUI, ItemDisplayContext.GROUND}) {
            ItemStackRenderState st = new ItemStackRenderState();
            resolver.updateForTopItem(st, glintStone, ctx, null, null, 0);
            RecordingCollector col = new RecordingCollector();
            st.submit(new PoseStack(), col, 15728880, 0, 0);
            blockItemJson.put(ctx.getSerializedName(), evaluateItemSubmits(col.items, gameRenderState, computeFoilDecalPose, false));
        }
        {
            ItemStackRenderState st = new ItemStackRenderState();
            resolver.updateForTopItem(st, glintStone, ItemDisplayContext.GUI, null, null, 0);
            RecordingCollector col = new RecordingCollector();
            PoseStack ps = new PoseStack();
            ps.translate(8.0F, 8.0F, 0.0F);
            ps.scale(16.0F, -16.0F, 16.0F);
            st.submit(ps, col, 15728880, 0, 0);
            blockItemJson.put("gui_slot_pose", evaluateItemSubmits(col.items, gameRenderState, computeFoilDecalPose, false));
        }

        // 7. Special foil (FoilType.SPECIAL): minecraft:compass with enchantment_glint_override = true.
        ItemStack glintCompass = new ItemStack(Items.COMPASS);
        glintCompass.set(DataComponents.ENCHANTMENT_GLINT_OVERRIDE, true);
        Map<String, Object> specialFoilJson = new LinkedHashMap<>();
        for (ItemDisplayContext ctx : new ItemDisplayContext[] {
            ItemDisplayContext.GUI,
            ItemDisplayContext.FIRST_PERSON_RIGHT_HAND,
            ItemDisplayContext.FIRST_PERSON_LEFT_HAND,
            ItemDisplayContext.GROUND,
            ItemDisplayContext.FIXED,
            ItemDisplayContext.THIRD_PERSON_RIGHT_HAND,
            ItemDisplayContext.HEAD
        }) {
            ItemStackRenderState st = new ItemStackRenderState();
            resolver.updateForTopItem(st, glintCompass, ctx, null, null, 0);
            RecordingCollector col = new RecordingCollector();
            st.submit(new PoseStack(), col, 15728880, 0, 0);
            specialFoilJson.put(ctx.getSerializedName(), evaluateItemSubmits(col.items, gameRenderState, computeFoilDecalPose, true));
        }
        // Also check compass at GUI slot poses (scales 1, 2, 3), nonzero atlas slots, and a non-trivial world pose.
        for (int guiScale : new int[] {1, 2, 3}) {
            ItemStackRenderState st = new ItemStackRenderState();
            resolver.updateForTopItem(st, glintCompass, ItemDisplayContext.GUI, null, null, 0);
            RecordingCollector col = new RecordingCollector();
            PoseStack ps = new PoseStack();
            float slotSize = 16.0F * guiScale;
            ps.translate(slotSize / 2.0F, slotSize / 2.0F, 0.0F);
            ps.scale(slotSize, -slotSize, slotSize);
            st.submit(ps, col, 15728880, 0, 0);
            String key = guiScale == 1 ? "gui_slot_pose" : ("gui_slot_scale_" + guiScale);
            specialFoilJson.put(key, evaluateItemSubmits(col.items, gameRenderState, computeFoilDecalPose, true));
        }
        for (Object[] slotCase : new Object[][] {
            {"gui_slot_16_0_scale_1", 1, 16, 0},
            {"gui_slot_3_2_scale_2", 2, 3, 2}
        }) {
            int guiScale = (int) slotCase[1];
            int slotX = (int) slotCase[2];
            int slotY = (int) slotCase[3];
            ItemStackRenderState st = new ItemStackRenderState();
            resolver.updateForTopItem(st, glintCompass, ItemDisplayContext.GUI, null, null, 0);
            RecordingCollector col = new RecordingCollector();
            PoseStack ps = new PoseStack();
            int slotTextureSize = 16 * guiScale;
            int left = slotX * slotTextureSize;
            int top = slotY * slotTextureSize;
            ps.translate(left + slotTextureSize / 2.0F, top + slotTextureSize / 2.0F, 0.0F);
            ps.scale(slotTextureSize, -slotTextureSize, slotTextureSize);
            st.submit(ps, col, 15728880, 0, 0);
            specialFoilJson.put((String) slotCase[0], evaluateItemSubmits(col.items, gameRenderState, computeFoilDecalPose, true));
        }
        {
            ItemStackRenderState st = new ItemStackRenderState();
            resolver.updateForTopItem(st, glintCompass, ItemDisplayContext.GROUND, null, null, 0);
            RecordingCollector col = new RecordingCollector();
            PoseStack ps = new PoseStack();
            ps.translate(1.5F, 2.0F, -3.5F);
            ps.rotate(new Quaternionf().rotationY(0.7F));
            ps.scale(0.5F, 0.5F, 0.5F);
            st.submit(ps, col, 15728880, 0, 0);
            specialFoilJson.put("ground_posed", evaluateItemSubmits(col.items, gameRenderState, computeFoilDecalPose, true));
        }
        // Degenerate flattened compass [1, 1, 0]: non-finite inverse projection, never zeroed.
        for (Object[] fc : new Object[][] {
            {"flattened_gui", ItemDisplayContext.GUI},
            {"flattened_ground", ItemDisplayContext.GROUND}
        }) {
            ItemStackRenderState st = new ItemStackRenderState();
            resolver.updateForTopItem(st, glintCompass, (ItemDisplayContext) fc[1], null, null, 0);
            RecordingCollector col = new RecordingCollector();
            PoseStack ps = new PoseStack();
            ps.scale(1.0F, 1.0F, 0.0F);
            st.submit(ps, col, 15728880, 0, 0);
            specialFoilJson.put((String) fc[0], evaluateItemSubmits(col.items, gameRenderState, computeFoilDecalPose, false));
        }
        // All 6 face directions (stone's 6 faces) through SPECIAL decal projection, plus
        // scale [1e-21, 1, 1] where East/West transformed normals overflow normalization -> North.
        for (Object[] dc : new Object[][] {
            {"stone_six_dirs_gui", ItemDisplayContext.GUI, new float[] {1.0F, 1.0F, 1.0F}},
            {"stone_six_dirs_ground", ItemDisplayContext.GROUND, new float[] {1.0F, 1.0F, 1.0F}},
            {"stone_scale_1e21_ground", ItemDisplayContext.GROUND, new float[] {1e-21F, 1.0F, 1.0F}}
        }) {
            ItemStackRenderState st = new ItemStackRenderState();
            resolver.updateForTopItem(st, glintStone, (ItemDisplayContext) dc[1], null, null, 0);
            RecordingCollector col = new RecordingCollector();
            PoseStack ps = new PoseStack();
            float[] sc = (float[]) dc[2];
            if (sc[0] != 1.0F || sc[1] != 1.0F || sc[2] != 1.0F) {
                ps.scale(sc[0], sc[1], sc[2]);
            }
            st.submit(ps, col, 15728880, 0, 0);
            List<CapturedItemSubmit> asSpecial = col.items.stream().map(s -> new CapturedItemSubmit(
                s.pose(), s.displayContext(), s.lightCoords(), s.overlayCoords(),
                s.outlineColor(), s.tintLayers(), s.quads(), ItemStackRenderState.FoilType.SPECIAL
            )).toList();
            specialFoilJson.put((String) dc[0], evaluateItemSubmits(asSpecial, gameRenderState, computeFoilDecalPose, false));
        }

        // 8. Special entity-model items: trident (2D gui/ground, 3D hand) and plain/red shields.
        ItemStack glintTrident = new ItemStack(Items.TRIDENT);
        glintTrident.set(DataComponents.ENCHANTMENT_GLINT_OVERRIDE, true);
        ItemStack glintShieldPlain = new ItemStack(Items.SHIELD);
        glintShieldPlain.set(DataComponents.ENCHANTMENT_GLINT_OVERRIDE, true);
        ItemStack glintShieldRed = new ItemStack(Items.SHIELD);
        glintShieldRed.set(DataComponents.ENCHANTMENT_GLINT_OVERRIDE, true);
        glintShieldRed.set(DataComponents.BASE_COLOR, DyeColor.RED);
        ItemStack glintShieldRedStripe = new ItemStack(Items.SHIELD);
        glintShieldRedStripe.set(DataComponents.ENCHANTMENT_GLINT_OVERRIDE, true);
        glintShieldRedStripe.set(DataComponents.BASE_COLOR, DyeColor.RED);
        glintShieldRedStripe.set(
            DataComponents.BANNER_PATTERNS,
            new BannerPatternLayers(List.of(new BannerPatternLayers.Layer(
                Holder.direct(new BannerPattern(
                    Identifier.withDefaultNamespace("stripe_bottom"),
                    "block.minecraft.banner.stripe_bottom"
                )),
                DyeColor.BLUE
            )))
        );

        Map<String, Object> specialModelsJson = new LinkedHashMap<>();
        Object[][] specialCases = new Object[][] {
            {"trident", glintTrident, new ItemDisplayContext[] {
                ItemDisplayContext.GUI,
                ItemDisplayContext.GROUND,
                ItemDisplayContext.FIRST_PERSON_RIGHT_HAND,
                ItemDisplayContext.THIRD_PERSON_RIGHT_HAND
            }},
            {"shield_plain", glintShieldPlain, new ItemDisplayContext[] {
                ItemDisplayContext.GUI,
                ItemDisplayContext.GROUND
            }},
            {"shield_red", glintShieldRed, new ItemDisplayContext[] {
                ItemDisplayContext.GUI,
                ItemDisplayContext.GROUND
            }},
            {"shield_red_stripe", glintShieldRedStripe, new ItemDisplayContext[] {
                ItemDisplayContext.GROUND
            }}
        };
        for (Object[] sc : specialCases) {
            String key = (String) sc[0];
            ItemStack stack = (ItemStack) sc[1];
            ItemDisplayContext[] ctxs = (ItemDisplayContext[]) sc[2];
            Map<String, Object> perCtx = new LinkedHashMap<>();
            for (ItemDisplayContext ctx : ctxs) {
                ItemStackRenderState st = new ItemStackRenderState();
                resolver.updateForTopItem(st, stack, ctx, null, null, 0);
                RecordingCollector col = new RecordingCollector();
                st.submit(new PoseStack(), col, 15728880, 0, 0);
                perCtx.put(
                    ctx.getSerializedName(),
                    map(
                        "items", col.items.isEmpty() ? null : evaluateItemSubmits(col.items, gameRenderState, computeFoilDecalPose, true),
                        "models", evaluateModelSubmits(col.models, gameRenderState)
                    )
                );
            }
            specialModelsJson.put(key, perCtx);
        }
        {
            ItemStackRenderState st = new ItemStackRenderState();
            resolver.updateForTopItem(st, glintShieldPlain, ItemDisplayContext.GUI, null, null, 0);
            RecordingCollector col = new RecordingCollector();
            PoseStack ps = new PoseStack();
            ps.translate(8.0F, 8.0F, 0.0F);
            ps.scale(16.0F, -16.0F, 16.0F);
            st.submit(ps, col, 15728880, 0, 0);
            @SuppressWarnings("unchecked")
            Map<String, Object> shieldPlainMap = (Map<String, Object>) specialModelsJson.get("shield_plain");
            shieldPlainMap.put("gui_slot_pose", map("models", evaluateModelSubmits(col.models, gameRenderState)));
        }

        Map<String, Object> materialBakerJson;
        {
            TextureAtlasSprite vanillaMissing = materialBaker
                .get(new Material(Identifier.withDefaultNamespace("missingno")), () -> "oracle")
                .sprite();
            Path synthRoot = Files.createTempDirectory("sunrise-glint-material-baker-");
            try {
                Path atlasesDir = synthRoot.resolve("assets/minecraft/atlases");
                Path texDir = synthRoot.resolve("assets/test/textures");
                Files.createDirectories(atlasesDir);
                Files.createDirectories(texDir);
                Files.writeString(
                    atlasesDir.resolve("items.json"),
                    "{\"sources\":[{\"type\":\"single\",\"resource\":\"test:bad_item\",\"sprite\":\"test:shared\"}]}",
                    StandardCharsets.UTF_8
                );
                Files.writeString(
                    atlasesDir.resolve("blocks.json"),
                    "{\"sources\":[{\"type\":\"single\",\"resource\":\"test:good_block\",\"sprite\":\"test:shared\"},{\"type\":\"single\",\"resource\":\"test:big_block\"}]}",
                    StandardCharsets.UTF_8
                );
                Files.writeString(texDir.resolve("bad_item.png"), "not a png", StandardCharsets.UTF_8);
                try (NativeImage img = new NativeImage(16, 16, false)) {
                    img.fillRect(0, 0, 16, 16, 0xFFFFFFFF);
                    img.writeToFile(texDir.resolve("good_block.png"));
                }
                try (NativeImage img = new NativeImage(128, 128, false)) {
                    img.fillRect(0, 0, 128, 128, 0xFFFFFFFF);
                    img.writeToFile(texDir.resolve("big_block.png"));
                }
                var packInfo = new PackLocationInfo("synth", Component.literal("synth"), PackSource.BUILT_IN, Optional.empty());
                try (MultiPackResourceManager synthMgr = new MultiPackResourceManager(
                    PackType.CLIENT_RESOURCES,
                    List.of(new PathPackResources(packInfo, synthRoot))
                )) {
                    SpriteLoader.Preparations synthItems = new SpriteLoader(TextureAtlas.LOCATION_ITEMS, 16384)
                        .loadAndStitch(synthMgr, AtlasIds.ITEMS, 0, Runnable::run, Set.of())
                        .join();
                    SpriteLoader.Preparations synthBlocks = new SpriteLoader(TextureAtlas.LOCATION_BLOCKS, 16384)
                        .loadAndStitch(synthMgr, AtlasIds.BLOCKS, 0, Runnable::run, Set.of())
                        .join();
                    MaterialBaker synthBaker = new MaterialBaker(synthBlocks, synthItems);
                    TextureAtlasSprite shared = synthBaker
                        .get(new Material(Identifier.parse("test:shared")), () -> "oracle")
                        .sprite();
                    TextureAtlasSprite synthMissing = synthBaker
                        .get(new Material(Identifier.withDefaultNamespace("missingno")), () -> "oracle")
                        .sprite();
                    materialBakerJson = map(
                        "vanilla_missingno_bits", new long[] {
                            u32(vanillaMissing.getU0()),
                            u32(vanillaMissing.getV0()),
                            u32(vanillaMissing.getU1()),
                            u32(vanillaMissing.getV1())
                        },
                        "synth_items_has_shared", synthItems.getSprite(Identifier.parse("test:shared")) != null,
                        "synth_shared_atlas", shared.atlasLocation().toString(),
                        "synth_shared_bits", new long[] {
                            u32(shared.getU0()),
                            u32(shared.getV0()),
                            u32(shared.getU1()),
                            u32(shared.getV1())
                        },
                        "synth_missingno_atlas", synthMissing.atlasLocation().toString(),
                        "synth_missingno_bits", new long[] {
                            u32(synthMissing.getU0()),
                            u32(synthMissing.getV0()),
                            u32(synthMissing.getU1()),
                            u32(synthMissing.getV1())
                        }
                    );
                }
            } finally {
                try (var walk = Files.walk(synthRoot)) {
                    walk.sorted(Comparator.reverseOrder()).forEach(p -> {
                        try {
                            Files.deleteIfExists(p);
                        } catch (Exception ignored) {
                        }
                    });
                }
            }
        }

        // 9. Banner pattern pipeline + partially transparent texel (alpha 8/255) blending contract.
        RenderType bannerPatternRt = RenderTypes.bannerPattern(Sheets.SHIELD_SHEET);
        RenderSetup bannerPatternSetup = renderSetupOf(bannerPatternRt);
        Map<String, Object> bannerPatternJson = map(
            "render_type", renderTypeName(bannerPatternRt),
            "pipeline", describePipeline(RenderPipelines.BANNER_PATTERN),
            "force_solid_model_phase", (boolean) getField(bannerPatternSetup, RenderSetup.class, "forceSolidModelPhase"),
            "has_oit", getField(bannerPatternSetup, RenderSetup.class, "oitPipelineSet") != null,
            "alpha_8_src_rgba8", new int[] {255, 0, 0, 8},
            "base_dst_rgba8", new int[] {0, 200, 0, 255},
            "blended_rgba8", new int[] {
                Math.round(255.0F * (8.0F / 255.0F) + 0.0F * (1.0F - 8.0F / 255.0F)),
                Math.round(0.0F * (8.0F / 255.0F) + 200.0F * (1.0F - 8.0F / 255.0F)),
                0,
                255
            }
        );

        // 10. OIT translucent item foil pipelines (OIT_ITEM, OIT_ITEM_GLINT, OIT_ITEM_GLINT_SPECIAL).
        Map<String, Object> oitItemGlintJson = map(
            "oit_item", describeOitSet(RenderPipelines.OIT_ITEM),
            "oit_item_glint", describeOitSet(RenderPipelines.OIT_ITEM_GLINT),
            "oit_item_glint_special", describeOitSet(RenderPipelines.OIT_ITEM_GLINT_SPECIAL)
        );

        // 11. Physical GuiItemAtlas slot allocation and nonzero-slot decal rounding witness.
        Map<String, Object> allocatorByScale = new LinkedHashMap<>();
        for (int guiScale : new int[] {1, 2, 3}) {
            int slotTextureSize = 16 * guiScale;
            int storageSize = 512 / slotTextureSize;
            DynamicAtlasAllocator<String> alloc = new DynamicAtlasAllocator<>(storageSize, storageSize);
            for (int i = 0; i < 16; i++) {
                alloc.getOrAllocate("static_" + i, false);
            }
            DynamicAtlasAllocator.Slot foilA = alloc.getOrAllocate("foil_a", true);
            DynamicAtlasAllocator.Slot foilAReuse = alloc.getOrAllocate("foil_a", true);
            alloc.endFrame();
            DynamicAtlasAllocator.Slot static0After = alloc.getOrAllocate("static_0", false);
            DynamicAtlasAllocator.Slot foilB = alloc.getOrAllocate("foil_b", true);
            allocatorByScale.put(
                "scale_" + guiScale,
                map(
                    "slot_texture_size", slotTextureSize,
                    "storage_size", storageSize,
                    "foil_a", new int[] {foilA.x(), foilA.y()},
                    "foil_a_reuse", new int[] {foilAReuse.x(), foilAReuse.y()},
                    "static_0_after_end_frame", new int[] {static0After.x(), static0After.y()},
                    "foil_b_after_end_frame", new int[] {foilB.x(), foilB.y()}
                )
            );
        }
        Map<String, Object> tinyOffsetRounding = new LinkedHashMap<>();
        {
            ItemStackRenderState compassState = new ItemStackRenderState();
            resolver.updateForTopItem(compassState, glintCompass, ItemDisplayContext.GUI, null, null, 0);
            RecordingCollector compassCol = new RecordingCollector();
            compassState.submit(new PoseStack(), compassCol, 15728880, 0, 0);
            BakedQuad.MaterialInfo compassMat = compassCol.items.get(0).quads().all().get(0).materialInfo();
            float eps = Math.scalb(1.0F, -20);
            setClock(gameRenderState, 123456L, 0.5);
            Matrix4f glintMatT123456 = TextureTransform.GLINT_TEXTURING.createMatrix();
            QuadInstance qi = new QuadInstance();
            qi.setLightCoords(15728880);
            qi.setOverlayCoords(0);
            qi.setColor(-1);
            for (Object[] tc : new Object[][] {
                {"slot_0_0_eps", 0, 0, eps},
                {"slot_0_0_zero", 0, 0, 0.0F},
                {"slot_16_0_eps", 16, 0, eps},
                {"slot_16_0_zero", 16, 0, 0.0F}
            }) {
                int slotX = (int) tc[1];
                int slotY = (int) tc[2];
                float offset = (float) tc[3];
                BakedQuad synthQuad = new BakedQuad(
                    new Vector3f(offset, 0.25F, 0.0F),
                    new Vector3f(offset, -0.25F, 0.0F),
                    new Vector3f(0.25F, -0.25F, 0.0F),
                    new Vector3f(0.25F, 0.25F, 0.0F),
                    UVPair.pack(0.0F, 0.0F),
                    UVPair.pack(0.0F, 1.0F),
                    UVPair.pack(1.0F, 1.0F),
                    UVPair.pack(1.0F, 0.0F),
                    Direction.SOUTH,
                    compassMat
                );
                PoseStack ps = new PoseStack();
                int left = slotX * 16;
                int top = slotY * 16;
                ps.translate(left + 8.0F, top + 8.0F, 0.0F);
                ps.scale(16.0F, -16.0F, 16.0F);
                PoseStack.Pose decalPose = (PoseStack.Pose) computeFoilDecalPose.invoke(null, ItemDisplayContext.GUI, ps.last());
                RecordingConsumer rc = new RecordingConsumer();
                rc.putBakedQuadWithGlint(ps.last(), synthQuad, qi, decalPose);
                long[][] uv3Bits = new long[4][2];
                long[][] glintBits = new long[4][2];
                for (int i = 0; i < 4; i++) {
                    RecordedVertex rv = rc.vertices.get(i);
                    uv3Bits[i] = new long[] {u32(rv.u3), u32(rv.v3)};
                    glintBits[i] = transformUvBits(glintMatT123456, rv.u3, rv.v3);
                }
                tinyOffsetRounding.put(
                    (String) tc[0],
                    map("uv3_bits", uv3Bits, "glint_uv_t123456_bits", glintBits)
                );
            }
        }
        Map<String, Object> guiItemAtlasJson = map(
            "allocator", allocatorByScale,
            "tiny_offset_rounding", tinyOffsetRounding
        );

        Map<String, Object> root = map(
            "meta", map(
                "game_version", SharedConstants.getCurrentVersion().name(),
                "java_version", System.getProperty("java.version"),
                "oracle", "dev.sunriselab.parity.VanillaItemGlintOracle",
                "clocks", map("t0", map("millis", 0, "speed", 0.5), "t123456", map("millis", 123456, "speed", 0.5))
            ),
            "transforms", transformsJson,
            "atlases", atlasesJson,
            "material_baker", materialBakerJson,
            "end_crystal_item_display", eid83245Json,
            "contexts", contextsJson,
            "block_item", blockItemJson,
            "special_foil", specialFoilJson,
            "special_models", specialModelsJson,
            "banner_pattern", bannerPatternJson,
            "oit_item_glint", oitItemGlintJson,
            "gui_item_atlas", guiItemAtlasJson
        );

        Files.createDirectories(outPath.getParent());
        Files.writeString(outPath, JSON.toJson(root) + "\n", StandardCharsets.UTF_8);
        System.out.println("Wrote witness to " + outPath + " (" + Files.size(outPath) + " bytes)");
    }
}
