package dev.sunriselab.parity;

import com.google.gson.*;
import it.unimi.dsi.fastutil.longs.LongOpenHashSet;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.Future;
import java.util.concurrent.locks.ReentrantLock;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.SectionOcclusionGraph;
import net.minecraft.client.renderer.ViewArea;
import net.minecraft.client.renderer.chunk.CompiledSectionMesh;
import net.minecraft.client.renderer.chunk.SectionMesh;
import net.minecraft.client.renderer.chunk.SectionRenderDispatcher;
import net.minecraft.client.renderer.chunk.VisibilitySet;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.client.renderer.state.level.ChunkLoadingRenderState;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.SectionPos;
import net.minecraft.util.Util;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.phys.Vec3;
import org.joml.Matrix4f;

/**
 * Runs real Minecraft 26.3 SectionOcclusionGraph + ViewArea + Octree + Frustum on synthetic
 * scenarios to witness exact terrain section reachability and frustum-visible sections across
 * heights, render distances, smartCull settings, empty sections, and vis masks.
 *
 * Usage: VanillaSectionVisibilityOracle <cases.json> <output.json>
 */
public final class VanillaSectionVisibilityOracle {
    private static Object unsafe;
    private static Method allocate;

    static Object allocate(Class<?> type) throws Exception {
        return allocate.invoke(unsafe, type);
    }

    static Field findField(Class<?> owner, String name) throws Exception {
        for (Class<?> c = owner; c != null; c = c.getSuperclass()) {
            try {
                Field f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException ignored) {}
        }
        throw new NoSuchFieldException(owner.getName() + "." + name);
    }

    static void set(Class<?> owner, Object target, String name, Object value) throws Exception {
        findField(owner, name).set(target, value);
    }

    static Object get(Class<?> owner, Object target, String name) throws Exception {
        return findField(owner, name).get(target);
    }

    static SectionMesh makeMesh(long mask) throws Exception {
        long allVisible = (1L << 36) - 1L;
        if (mask == allVisible) {
            return CompiledSectionMesh.EMPTY;
        }
        VisibilitySet vis = new VisibilitySet();
        Direction[] dirs = Direction.values();
        for (int a = 0; a < 6; a++) {
            for (int b = 0; b < 6; b++) {
                if (((mask >>> (a * 6 + b)) & 1L) != 0L) {
                    vis.set(dirs[a], dirs[b], true);
                }
            }
        }
        CompiledSectionMesh mesh = (CompiledSectionMesh) allocate(CompiledSectionMesh.class);
        set(CompiledSectionMesh.class, mesh, "visibilitySet", vis);
        set(CompiledSectionMesh.class, mesh, "draws", Map.of());
        set(CompiledSectionMesh.class, mesh, "renderableBlockEntities", List.of());
        return mesh;
    }

    static Frustum makeFrustum(double camX, double camY, double camZ, float yaw, float pitch, float fovDeg, float aspect, float far) throws Exception {
        Camera cam = (Camera) allocate(Camera.class);
        set(Camera.class, cam, "rotation", new org.joml.Quaternionf());
        set(Camera.class, cam, "forwards", new org.joml.Vector3f(0.0F, 0.0F, 1.0F));
        set(Camera.class, cam, "up", new org.joml.Vector3f(0.0F, 1.0F, 0.0F));
        set(Camera.class, cam, "left", new org.joml.Vector3f(1.0F, 0.0F, 0.0F));
        set(Camera.class, cam, "cachedViewRotMatrix", new Matrix4f());
        Method setRot = Camera.class.getDeclaredMethod("setRotation", float.class, float.class);
        setRot.setAccessible(true);
        setRot.invoke(cam, yaw, pitch);
        Matrix4f viewRot = cam.getViewRotationMatrix(new Matrix4f());
        Matrix4f proj = new Matrix4f().perspective(fovDeg * (float) (Math.PI / 180.0), aspect, 0.05F, far, true);
        Frustum frustum = new Frustum(viewRot, proj);
        frustum.prepare(camX, camY, camZ);
        return frustum;
    }

    static JsonArray sectionCoords(Collection<long[]> coords) {
        List<int[]> sorted = new ArrayList<>();
        for (long[] s : coords) {
            sorted.add(new int[] { SectionPos.x(s[0]), SectionPos.y(s[0]), SectionPos.z(s[0]) });
        }
        sorted.sort(Comparator.<int[]>comparingInt(a -> a[0]).thenComparingInt(a -> a[1]).thenComparingInt(a -> a[2]));
        JsonArray arr = new JsonArray();
        for (int[] c : sorted) {
            JsonArray triple = new JsonArray();
            triple.add(c[0]);
            triple.add(c[1]);
            triple.add(c[2]);
            arr.add(triple);
        }
        return arr;
    }

    static JsonArray sectionNodesToJson(Collection<Long> nodes) {
        List<int[]> sorted = new ArrayList<>();
        for (long node : nodes) {
            sorted.add(new int[] { SectionPos.x(node), SectionPos.y(node), SectionPos.z(node) });
        }
        sorted.sort(Comparator.<int[]>comparingInt(a -> a[0]).thenComparingInt(a -> a[1]).thenComparingInt(a -> a[2]));
        JsonArray arr = new JsonArray();
        for (int[] c : sorted) {
            JsonArray triple = new JsonArray();
            triple.add(c[0]);
            triple.add(c[1]);
            triple.add(c[2]);
            arr.add(triple);
        }
        return arr;
    }

    public static void main(String[] args) throws Exception {
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);

        Minecraft minecraft = (Minecraft) allocate(Minecraft.class);
        set(Minecraft.class, minecraft, "gameThread", Thread.currentThread());
        Object levelRenderer = allocate(net.minecraft.client.renderer.LevelRenderer.class);
        set(
            net.minecraft.client.renderer.LevelRenderer.class,
            levelRenderer,
            "transientBlockRemovalQueue",
            new java.util.concurrent.LinkedBlockingQueue<>()
        );
        set(Minecraft.class, minecraft, "levelRenderer", levelRenderer);
        set(Minecraft.class, null, "instance", minecraft);

        SectionRenderDispatcher dispatcher = (SectionRenderDispatcher) allocate(SectionRenderDispatcher.class);
        set(SectionRenderDispatcher.class, dispatcher, "copyLock", new ReentrantLock());
        set(SectionRenderDispatcher.class, dispatcher, "chunkUberBuffers", Map.of());

        Method getRenderSection = ViewArea.class.getDeclaredMethod("getRenderSection", long.class);
        getRenderSection.setAccessible(true);
        Method runPartialUpdate = SectionOcclusionGraph.class.getDeclaredMethod(
            "runPartialUpdate",
            CameraRenderState.class,
            it.unimi.dsi.fastutil.longs.LongSet.class
        );
        runPartialUpdate.setAccessible(true);

        JsonObject spec = JsonParser.parseString(Files.readString(Path.of(args[0]), StandardCharsets.UTF_8)).getAsJsonObject();
        JsonArray outCases = new JsonArray();

        for (JsonElement elem : spec.getAsJsonArray("cases")) {
            JsonObject c = elem.getAsJsonObject();
            String name = c.get("name").getAsString();
            int minY = c.get("min_y").getAsInt();
            int sections = c.get("sections").getAsInt();
            int maxY = minY + sections * 16 - 1;
            int minSecY = SectionPos.blockToSectionCoord(minY);
            int maxSecY = SectionPos.blockToSectionCoord(maxY);
            int rd = c.get("render_distance").getAsInt();
            JsonArray camArr = c.getAsJsonArray("camera");
            double camX = camArr.get(0).getAsDouble();
            double camY = camArr.get(1).getAsDouble();
            double camZ = camArr.get(2).getAsDouble();
            Vec3 camPos = new Vec3(camX, camY, camZ);
            boolean smartCull = c.get("smart_cull").getAsBoolean();

            SectionOcclusionGraph graph = new SectionOcclusionGraph();
            ViewArea viewArea = new ViewArea(dispatcher, minY, maxY, minSecY, maxSecY, rd, graph);
            graph.waitAndReset(viewArea);
            SectionPos camSec = SectionPos.of(camPos);
            viewArea.repositionCamera(camSec);

            Set<Integer> nonEmptyYs = null;
            if (c.has("non_empty_world_sy")) {
                nonEmptyYs = new HashSet<>();
                for (JsonElement e : c.getAsJsonArray("non_empty_world_sy")) {
                    nonEmptyYs.add(e.getAsInt());
                }
            }

            Map<Long, Long> visOverrides = new HashMap<>();
            if (c.has("vis_overrides")) {
                for (JsonElement e : c.getAsJsonArray("vis_overrides")) {
                    JsonObject o = e.getAsJsonObject();
                    JsonArray s = o.getAsJsonArray("section");
                    long node = SectionPos.asLong(s.get(0).getAsInt(), s.get(1).getAsInt(), s.get(2).getAsInt());
                    visOverrides.put(node, o.get("mask").getAsLong());
                }
            }

            ChunkLoadingRenderState loading = new ChunkLoadingRenderState();
            int camCx = camSec.x();
            int camCz = camSec.z();

            Set<Long> excludedChunks = new HashSet<>();
            if (c.has("excluded_loaded_chunks")) {
                for (JsonElement e : c.getAsJsonArray("excluded_loaded_chunks")) {
                    JsonArray pair = e.getAsJsonArray();
                    excludedChunks.add(ChunkPos.pack(pair.get(0).getAsInt(), pair.get(1).getAsInt()));
                }
            }

            Set<Long> loadedSet = new HashSet<>();
            if (c.has("loaded_chunk_range")) {
                JsonArray r = c.getAsJsonArray("loaded_chunk_range");
                for (int cx = r.get(0).getAsInt(); cx <= r.get(1).getAsInt(); cx++) {
                    for (int cz = r.get(2).getAsInt(); cz <= r.get(3).getAsInt(); cz++) {
                        long chunk = ChunkPos.pack(cx, cz);
                        if (excludedChunks.contains(chunk)) continue;
                        loadedSet.add(chunk);
                        loading.addedLoadedChunks.add(chunk);
                    }
                }
            } else {
                for (int cx = camCx - rd - 1; cx <= camCx + rd + 1; cx++) {
                    for (int cz = camCz - rd - 1; cz <= camCz + rd + 1; cz++) {
                        long chunk = ChunkPos.pack(cx, cz);
                        if (excludedChunks.contains(chunk)) continue;
                        loadedSet.add(chunk);
                        loading.addedLoadedChunks.add(chunk);
                    }
                }
            }

            for (int sx = camCx - rd; sx <= camCx + rd; sx++) {
                for (int sz = camCz - rd; sz <= camCz + rd; sz++) {
                    for (int sy = minSecY; sy <= maxSecY; sy++) {
                        long node = SectionPos.asLong(sx, sy, sz);
                        SectionRenderDispatcher.RenderSection rs =
                            (SectionRenderDispatcher.RenderSection) getRenderSection.invoke(viewArea, node);
                        if (rs == null) continue;
                        boolean isEmpty = (nonEmptyYs != null) && !nonEmptyYs.contains(sy) && !visOverrides.containsKey(node);
                        if (isEmpty) {
                            loading.addedEmptySections.add(node);
                            rs.sectionMesh.set(CompiledSectionMesh.UNCOMPILED);
                        } else {
                            long mask = visOverrides.getOrDefault(node, (1L << 36) - 1L);
                            rs.sectionMesh.set(makeMesh(mask));
                        }
                    }
                }
            }

            CameraRenderState camState = new CameraRenderState();
            camState.pos = camPos;
            camState.blockPos = BlockPos.containing(camPos);
            camState.smartCull = smartCull;
            camState.isFrustumCaptured = false;
            float far = Math.max(rd * 16.0F * 4.0F, 12.0F * 16.0F);
            camState.cullFrustum = makeFrustum(camX, camY, camZ, 0.0F, 0.0F, 70.0F, 1280.0F / 800.0F, far);

            graph.update(camState, 70, loading);
            Future<?> task = (Future<?>) get(SectionOcclusionGraph.class, graph, "fullUpdateTask");
            if (task != null) {
                task.get();
            }

            Set<Long> reached = new HashSet<>();
            for (int sx = camCx - rd; sx <= camCx + rd; sx++) {
                for (int sz = camCz - rd; sz <= camCz + rd; sz++) {
                    for (int sy = minSecY; sy <= maxSecY; sy++) {
                        long node = SectionPos.asLong(sx, sy, sz);
                        SectionRenderDispatcher.RenderSection rs =
                            (SectionRenderDispatcher.RenderSection) getRenderSection.invoke(viewArea, node);
                        if (rs != null && graph.getNode(rs) != null) {
                            reached.add(node);
                        }
                    }
                }
            }

            // Collect every section in Octree across all 6 cardinal view directions (90 deg, aspect 1.0, huge far).
            Set<Long> octreeAll = new HashSet<>();
            float[][] sixDirs = {
                {0.0F, 0.0F}, {90.0F, 0.0F}, {180.0F, 0.0F}, {270.0F, 0.0F},
                {0.0F, -90.0F}, {0.0F, 90.0F}
            };
            float bigFar = Math.max(far, 8192.0F);
            for (float[] dir : sixDirs) {
                Frustum f = makeFrustum(camX, camY, camZ, dir[0], dir[1], 95.0F, 1.0F, bigFar);
                List<SectionRenderDispatcher.RenderSection> vis = new ArrayList<>();
                List<SectionRenderDispatcher.RenderSection> nearby = new ArrayList<>();
                graph.addSectionsInFrustum(f, vis, nearby);
                for (SectionRenderDispatcher.RenderSection rs : vis) {
                    octreeAll.add(rs.getSectionNode());
                }
            }

            JsonObject out = new JsonObject();
            out.addProperty("name", name);
            out.addProperty("min_y", minY);
            out.addProperty("sections", sections);
            out.addProperty("min_section_y", minSecY);
            out.addProperty("max_section_y", maxSecY);
            out.addProperty("render_distance", rd);
            out.addProperty("smart_cull", smartCull);
            JsonArray camOut = new JsonArray();
            camOut.add(camX);
            camOut.add(camY);
            camOut.add(camZ);
            out.add("camera", camOut);
            JsonArray camBits = new JsonArray();
            camBits.add(String.format("%016x", Double.doubleToRawLongBits(camX)));
            camBits.add(String.format("%016x", Double.doubleToRawLongBits(camY)));
            camBits.add(String.format("%016x", Double.doubleToRawLongBits(camZ)));
            out.add("camera_bits", camBits);
            JsonArray camSecArr = new JsonArray();
            camSecArr.add(camSec.x());
            camSecArr.add(camSec.y());
            camSecArr.add(camSec.z());
            out.add("camera_section", camSecArr);
            out.addProperty("reached_count", reached.size());
            out.add("reached_sections", sectionNodesToJson(reached));
            out.addProperty("octree_all_count", octreeAll.size());
            out.add("octree_all_sections", sectionNodesToJson(octreeAll));

            if (c.has("frustum")) {
                JsonObject fo = c.getAsJsonObject("frustum");
                float yaw = fo.get("yaw").getAsFloat();
                float pitch = fo.get("pitch").getAsFloat();
                float fov = fo.get("fov").getAsFloat();
                float aspect = fo.get("aspect").getAsFloat();
                float frustumFar = fo.has("far") ? fo.get("far").getAsFloat() : far;
                Frustum f = makeFrustum(camX, camY, camZ, yaw, pitch, fov, aspect, frustumFar);
                List<SectionRenderDispatcher.RenderSection> vis = new ArrayList<>();
                List<SectionRenderDispatcher.RenderSection> nearby = new ArrayList<>();
                graph.addSectionsInFrustum(f, vis, nearby);
                Set<Long> visNodes = new HashSet<>();
                for (SectionRenderDispatcher.RenderSection rs : vis) {
                    visNodes.add(rs.getSectionNode());
                }
                out.addProperty("frustum_visible_count", visNodes.size());
                out.add("frustum_visible_sections", sectionNodesToJson(visNodes));
            }

            if (c.has("vis_overrides")) {
                out.add("vis_overrides", c.get("vis_overrides"));
            }
            if (c.has("non_empty_world_sy")) {
                out.add("non_empty_world_sy", c.get("non_empty_world_sy"));
            }
            if (c.has("excluded_loaded_chunks")) {
                out.add("excluded_loaded_chunks", c.get("excluded_loaded_chunks"));
            }
            if (c.has("loaded_chunk_range")) {
                out.add("loaded_chunk_range", c.get("loaded_chunk_range"));
            }
            if (c.has("frustum")) {
                out.add("frustum", c.get("frustum"));
            }
            if (c.has("tight_bounds")) {
                out.add("tight_bounds", c.get("tight_bounds"));
            }
            if (c.has("partial_step")) {
                JsonObject ps = c.getAsJsonObject("partial_step");
                double nextCamX = camX;
                double nextCamY = camY;
                double nextCamZ = camZ;
                if (ps.has("camera")) {
                    JsonArray pa = ps.getAsJsonArray("camera");
                    nextCamX = pa.get(0).getAsDouble();
                    nextCamY = pa.get(1).getAsDouble();
                    nextCamZ = pa.get(2).getAsDouble();
                    Vec3 nextCamPos = new Vec3(nextCamX, nextCamY, nextCamZ);
                    SectionPos nextCamSec = SectionPos.of(nextCamPos);
                    viewArea.repositionCamera(nextCamSec);
                    camState.pos = nextCamPos;
                    camState.blockPos = BlockPos.containing(nextCamPos);
                    camState.cullFrustum = makeFrustum(nextCamX, nextCamY, nextCamZ, 0.0F, 0.0F, 70.0F, 1280.0F / 800.0F, far);
                }
                LongOpenHashSet loadedExpected = new LongOpenHashSet();
                LongOpenHashSet graphLoaded = (LongOpenHashSet) get(SectionOcclusionGraph.class, graph, "loadedChunks");
                if (ps.has("add_loaded_chunks")) {
                    for (JsonElement e : ps.getAsJsonArray("add_loaded_chunks")) {
                        JsonArray pair = e.getAsJsonArray();
                        long chunk = ChunkPos.pack(pair.get(0).getAsInt(), pair.get(1).getAsInt());
                        graphLoaded.add(chunk);
                        loadedExpected.add(chunk);
                    }
                }
                if (ps.has("vis_overrides")) {
                    for (JsonElement e : ps.getAsJsonArray("vis_overrides")) {
                        JsonObject o = e.getAsJsonObject();
                        JsonArray s = o.getAsJsonArray("section");
                        long node = SectionPos.asLong(s.get(0).getAsInt(), s.get(1).getAsInt(), s.get(2).getAsInt());
                        SectionRenderDispatcher.RenderSection rs =
                            (SectionRenderDispatcher.RenderSection) getRenderSection.invoke(viewArea, node);
                        if (rs != null) {
                            rs.sectionMesh.set(makeMesh(o.get("mask").getAsLong()));
                            graph.schedulePropagationFrom(rs);
                        }
                    }
                }
                runPartialUpdate.invoke(graph, camState, loadedExpected);
                Set<Long> afterOctree = new HashSet<>();
                for (float[] dir : sixDirs) {
                    Frustum f = makeFrustum(nextCamX, nextCamY, nextCamZ, dir[0], dir[1], 95.0F, 1.0F, bigFar);
                    List<SectionRenderDispatcher.RenderSection> vis = new ArrayList<>();
                    List<SectionRenderDispatcher.RenderSection> nearby = new ArrayList<>();
                    graph.addSectionsInFrustum(f, vis, nearby);
                    for (SectionRenderDispatcher.RenderSection rs : vis) {
                        afterOctree.add(rs.getSectionNode());
                    }
                }
                out.add("partial_step", ps);
                out.addProperty("after_partial_octree_all_count", afterOctree.size());
                out.add("after_partial_octree_all_sections", sectionNodesToJson(afterOctree));
            }

            outCases.add(out);
        }

        JsonObject root = new JsonObject();
        JsonObject meta = new JsonObject();
        meta.addProperty("game_version", "26.3");
        meta.addProperty("java_version", System.getProperty("java.version"));
        meta.addProperty("oracle", "dev.sunriselab.parity.VanillaSectionVisibilityOracle");
        root.add("meta", meta);
        root.add("cases", outCases);

        Gson gson = new Gson();
        Files.writeString(Path.of(args[1]), gson.toJson(root) + "\n", StandardCharsets.UTF_8);
        Util.shutdownExecutors();
    }
}
