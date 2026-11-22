package dev.sunriselab.parity;

import com.mojang.serialization.Lifecycle;
import java.util.List;
import net.minecraft.client.multiplayer.ClientChunkCache;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.prediction.BlockStatePredictionHandler;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.Biomes;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.ObserverBlock;
import net.minecraft.world.level.block.RepeaterBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.chunk.PalettedContainerFactory;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.dimension.BuiltinDimensionTypes;
import net.minecraft.world.level.dimension.DimensionType;
import net.minecraft.world.level.redstone.Orientation;

/** Runs the shipped client placement and neighbor methods without a game window. */
public final class VanillaClientRedstonePreviewOracle {
    // Supply a real loaded chunk without connecting or loading a renderer.
    static final class Chunks extends ClientChunkCache {
        final LevelChunk chunk;
        Chunks(World world) {
            super(world, 0);
            chunk = new LevelChunk(world, new ChunkPos(0, 0));
        }
        @Override public LevelChunk getChunk(int x, int z, ChunkStatus status, boolean load) {
            return x == 0 && z == 0 ? chunk : super.getChunk(x, z, status, load);
        }
    }

    static final class World extends ClientLevel {
        DimensionType dimension;
        RegistryAccess registries;
        PalettedContainerFactory palettes;
        Chunks chunks;
        int comparatorNotifications;
        World() { super(null, null, null, null, 0, 0, null, false, 0, 0); }
        @Override public boolean isClientSide() { return true; }
        @Override public boolean hasNeighborSignal(BlockPos pos) { return true; }
        @Override public DimensionType dimensionType() { return dimension; }
        @Override public RegistryAccess registryAccess() { return registries; }
        @Override public PalettedContainerFactory palettedContainerFactory() { return palettes; }
        @Override public ClientChunkCache getChunkSource() { return chunks; }
        @Override public void setBlocksDirty(BlockPos pos, BlockState old, BlockState state) {}
        @Override public void neighborChanged(BlockState state, BlockPos pos, Block block,
                                              Orientation orientation, boolean movedByPiston) {
            if (state.is(Blocks.COMPARATOR)) comparatorNotifications++;
        }
        @Override public void neighborChanged(BlockPos pos, Block block, Orientation orientation) {
            if (getBlockState(pos).is(Blocks.COMPARATOR)) comparatorNotifications++;
        }
        @Override public boolean removeBlock(BlockPos pos, boolean moved) {
            throw new AssertionError("client placement must not remove powered TNT");
        }
        @Override public void blockEvent(BlockPos pos, Block block, int event, int direction) {
            throw new AssertionError("client placement must not start a piston move");
        }

        // Set up a snapshot; the subsequent write goes through ClientLevel, Level and LevelChunk.
        void seed(BlockPos pos, BlockState state) {
            chunks.chunk.getSection(chunks.chunk.getSectionIndex(pos.getY()))
                .setBlockState(pos.getX(), pos.getY(), pos.getZ(), state);
        }
    }

    private static void checkComparatorWrite(World world, BlockPos source, BlockState old,
                                             BlockState next, boolean throughConductor, String name) {
        world.chunks = new Chunks(world);
        world.comparatorNotifications = 0;
        var comparatorPos = throughConductor ? source.east(2) : source.east();
        var comparator = Blocks.COMPARATOR.defaultBlockState();
        world.seed(source, old);
        if (throughConductor) world.seed(source.east(), Blocks.STONE.defaultBlockState());
        world.seed(comparatorPos, comparator);
        if (!world.setBlock(source, next, Block.UPDATE_NEIGHBORS | Block.UPDATE_KNOWN_SHAPE)
                || world.getBlockState(source) != next || world.getBlockState(comparatorPos) != comparator
                || world.comparatorNotifications != 0) {
            throw new AssertionError(name + ": client must write the source without notifying the comparator");
        }
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var field = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        field.setAccessible(true);
        var unsafe = field.get(null);
        var allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        var world = (World) allocate.invoke(unsafe, World.class);
        var predictionHandler = ClientLevel.class.getDeclaredField("blockStatePredictionHandler");
        predictionHandler.setAccessible(true);
        predictionHandler.set(world, new BlockStatePredictionHandler());

        var lookup = VanillaRegistries.createWorldLookup();
        var biomes = new MappedRegistry<Biome>(Registries.BIOME, Lifecycle.stable());
        Registry.register(biomes, Biomes.PLAINS, lookup.lookupOrThrow(Registries.BIOME).getOrThrow(Biomes.PLAINS).value());
        world.dimension = lookup.lookupOrThrow(Registries.DIMENSION_TYPE).getOrThrow(BuiltinDimensionTypes.OVERWORLD).value();
        world.registries = new RegistryAccess.ImmutableRegistryAccess(List.of(biomes)).freeze();
        world.palettes = PalettedContainerFactory.create(world.registries);
        world.chunks = new Chunks(world);

        var pos = new BlockPos(4, 1, 4);
        var air = Blocks.AIR.defaultBlockState();
        for (var block : List.of(Blocks.TNT, Blocks.PISTON, Blocks.STICKY_PISTON)) {
            block.defaultBlockState().onPlace(world, pos, air, false);
        }

        var observer = Blocks.OBSERVER.defaultBlockState().setValue(ObserverBlock.POWERED, true);
        if (!world.setBlock(pos, observer, Block.UPDATE_MOVE_BY_PISTON | Block.UPDATE_KNOWN_SHAPE)
                || world.getBlockState(pos) != observer) {
            throw new AssertionError("client placement must preserve a powered observer");
        }

        world.chunks = new Chunks(world);
        var sponge = Blocks.SPONGE.defaultBlockState();
        var water = Blocks.WATER.defaultBlockState();
        world.seed(pos.east(), water);
        if (!world.setBlock(pos, sponge, Block.UPDATE_KNOWN_SHAPE)
                || world.getBlockState(pos) != sponge || world.getBlockState(pos.east()) != water) {
            throw new AssertionError("client chunk write must skip SpongeBlock.onPlace");
        }

        var repeater = Blocks.REPEATER.defaultBlockState()
            .setValue(RepeaterBlock.FACING, Direction.NORTH).setValue(RepeaterBlock.LOCKED, true);
        var updated = repeater.updateShape(world, world, pos, Direction.WEST,
            pos.west(), air, RandomSource.create(0));
        if (updated != repeater) {
            throw new AssertionError("a client side-neighbor shape update must preserve repeater lock");
        }

        var stone = Blocks.STONE.defaultBlockState();
        checkComparatorWrite(world, pos, air, stone, false, "plain adjacent notification");
        checkComparatorWrite(world, pos, Blocks.CHEST.defaultBlockState(), stone, true, "old source removal");
        checkComparatorWrite(world, pos, air, Blocks.CAKE.defaultBlockState(), true, "new analog source");
        System.out.println("Vanilla client placement and redstone update checks passed.");
    }
}
