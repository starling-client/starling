package dev.sunriselab.parity;

import com.google.gson.*;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.zip.ZipFile;
import com.mojang.serialization.JsonOps;
import net.minecraft.SharedConstants;
import net.minecraft.core.Holder;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.tags.TagFile;
import net.minecraft.tags.TagKey;
import net.minecraft.tags.TagLoader;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.SpawnPlacementTypes;
import net.minecraft.world.entity.SpawnPlacements;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.pathfinder.PathComputationType;
import net.minecraft.world.level.pathfinder.PathType;
import net.minecraft.world.level.pathfinder.WalkNodeEvaluator;

/**
 * Original exporter of mob AI rules from the server jar:
 * per-state pathfinding types (WalkNodeEvaluator.getPathTypeFromState), pathfindability,
 * spawn-surface predicates and each entity type's spawn placement.
 */
@SuppressWarnings("unchecked")
public final class VanillaMobDataExporter {
    /** A level holding one block state at the origin and air elsewhere. */
    private record OneBlock(BlockState state) implements BlockGetter {
        public BlockEntity getBlockEntity(BlockPos pos) { return null; }
        public BlockState getBlockState(BlockPos pos) { return pos.equals(BlockPos.ZERO) ? state : Blocks.AIR.defaultBlockState(); }
        public FluidState getFluidState(BlockPos pos) { return getBlockState(pos).getFluidState(); }
        public int getHeight() { return 384; }
        public int getMinY() { return -64; }
    }

    private static JsonArray states() throws Exception {
        Method pathType = WalkNodeEvaluator.class.getDeclaredMethod("getPathTypeFromState", BlockGetter.class, BlockPos.class);
        pathType.setAccessible(true);
        var out = new JsonArray();
        var empty = EmptyBlockGetter.INSTANCE;
        for (var state : Block.BLOCK_STATE_REGISTRY) {
            var level = new OneBlock(state);
            var type = (PathType) pathType.invoke(null, level, BlockPos.ZERO);
            var shape = state.getCollisionShape(empty, BlockPos.ZERO);
            var o = new JsonArray();
            o.add(type.name());
            int flags = 0;
            if (state.isPathfindable(PathComputationType.LAND)) flags |= 1;
            if (state.isPathfindable(PathComputationType.WATER)) flags |= 2;
            if (state.isPathfindable(PathComputationType.AIR)) flags |= 4;
            if (state.isValidSpawn(empty, BlockPos.ZERO, EntityTypes.ZOMBIE)) flags |= 8;
            if (state.isValidSpawn(empty, BlockPos.ZERO, EntityTypes.OCELOT)) flags |= 16;
            if (state.isValidSpawn(empty, BlockPos.ZERO, EntityTypes.POLAR_BEAR)) flags |= 32;
            if (state.isRedstoneConductor(empty, BlockPos.ZERO)) flags |= 64;
            if (net.minecraft.world.level.NaturalSpawner.isValidEmptySpawnBlock(level, BlockPos.ZERO, state, state.getFluidState(), EntityTypes.ZOMBIE)) flags |= 128;
            if (state.isFaceSturdy(empty, BlockPos.ZERO, Direction.UP)) flags |= 256;
            if (state.isSolid()) flags |= 512;
            if (state.isSolidRender()) flags |= 1024;
            o.add(flags);
            o.add(shape.isEmpty() ? 0.0 : shape.max(Direction.Axis.Y));
            out.add(o);
        }
        return out;
    }

    private static JsonObject placements() throws ReflectiveOperationException {
        var field = SpawnPlacements.class.getDeclaredField("DATA_BY_TYPE");
        field.setAccessible(true);
        var registrations = (Map<?, ?>) field.get(null);
        var out = new JsonObject();
        for (var type : BuiltInRegistries.ENTITY_TYPE) {
            var placement = SpawnPlacements.getPlacementType(type);
            String name = placement == SpawnPlacementTypes.ON_GROUND ? "on_ground"
                : placement == SpawnPlacementTypes.IN_WATER ? "in_water"
                : placement == SpawnPlacementTypes.IN_LAVA ? "in_lava" : "no_restrictions";
            var o = new JsonObject();
            o.addProperty("placement", name);
            o.addProperty("heightmap", SpawnPlacements.getHeightmapType(type).getSerializedName());
            o.addProperty("registered", registrations.containsKey(type));
            out.add(BuiltInRegistries.ENTITY_TYPE.getKey(type).toString(), o);
        }
        return out;
    }

    /** Binds one registry's vanilla tags from the jar's data pack, as a server reload would. */
    private static <T> void bindTags(ZipFile jar, Registry<T> registry, ResourceKey<? extends Registry<T>> key, String folder) throws Exception {
        var loader = new TagLoader<Holder<T>>((id, required) -> registry.get(id).map(value -> (Holder<T>) value), "tags/" + folder);
        var definitions = new HashMap<Identifier, List<TagLoader.EntryWithSource>>();
        var prefix = "data/minecraft/tags/" + folder + "/";
        var entries = jar.entries();
        while (entries.hasMoreElements()) {
            var entry = entries.nextElement();
            var name = entry.getName();
            if (!name.startsWith(prefix) || !name.endsWith(".json")) continue;
            var text = new String(jar.getInputStream(entry).readAllBytes(), StandardCharsets.UTF_8);
            var file = TagFile.CODEC.parse(JsonOps.INSTANCE, JsonParser.parseString(text)).getOrThrow();
            var list = new ArrayList<TagLoader.EntryWithSource>();
            for (var e : file.entries()) list.add(new TagLoader.EntryWithSource(e, "vanilla"));
            definitions.put(Identifier.parse("minecraft:" + name.substring(prefix.length(), name.length() - 5)), list);
        }
        var tags = new HashMap<TagKey<T>, List<Holder<T>>>();
        loader.build(definitions).forEach((id, values) -> tags.put(TagKey.create(key, id), List.copyOf(values)));
        registry.prepareTagReload(new TagLoader.LoadResult<>(key, tags)).apply();
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        try (var jar = new ZipFile(args[1])) {
            bindTags(jar, BuiltInRegistries.BLOCK, Registries.BLOCK, "block");
            bindTags(jar, BuiltInRegistries.FLUID, Registries.FLUID, "fluid");
        }
        var result = new JsonObject();
        result.add("states", states());
        result.add("placements", placements());
        Files.writeString(Path.of(args[0]), new Gson().toJson(result), StandardCharsets.UTF_8);
    }
}
