package dev.sunriselab.parity;

import io.netty.buffer.Unpooled;
import it.unimi.dsi.fastutil.shorts.ShortList;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.BitSet;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.SectionPos;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.protocol.game.ClientboundLightUpdatePacketData;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.DataLayer;
import net.minecraft.world.level.chunk.LightChunk;
import net.minecraft.world.level.chunk.LightChunkGetter;
import net.minecraft.world.level.chunk.PalettedContainerFactory;
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.chunk.storage.SerializableChunkData;
import net.minecraft.world.level.lighting.ChunkSkyLightSources;
import net.minecraft.world.level.lighting.LayerLightSectionStorage.SectionType;
import net.minecraft.world.level.lighting.LevelLightEngine;

/**
 * Replays tools/light_storage_scenario.txt on the real LevelLightEngine, as a client drives it
 * (ClientPacketListener: updateSectionStatus per section, readSectionList, setLightEnabled; forgetting a
 * column is queueLightRemoval), and prints the sky and block light of every read.
 */
public final class VanillaLightStorageOracle {
    private static final int MIN_SECTION = -4;
    private static final int SECTIONS = 24;

    static LevelLightEngine engine(boolean sky, LightChunk center) {
        BlockGetter level = new BlockGetter() {
            public net.minecraft.world.level.block.entity.BlockEntity getBlockEntity(BlockPos pos) { return null; }
            public net.minecraft.world.level.block.state.BlockState getBlockState(BlockPos pos) { return null; }
            public net.minecraft.world.level.material.FluidState getFluidState(BlockPos pos) { return null; }
            public int getHeight() { return SECTIONS * 16; }
            public int getMinY() { return MIN_SECTION * 16; }
        };
        LightChunkGetter getter = new LightChunkGetter() {
            public LightChunk getChunkForLighting(int x, int z) { return x == 0 && z == 0 ? center : null; }
            public BlockGetter getLevel() { return level; }
        };
        return new LevelLightEngine(getter, true, sky);
    }

    private static int value(String spec, int x, int y, int z) {
        int n = Integer.parseInt(spec.substring(1));
        return spec.charAt(0) == 'U' ? n : (x + 2 * z + 3 * y + n) & 15;
    }

    private static DataLayer layer(String spec) {
        DataLayer layer = new DataLayer();
        for (int y = 0; y < 16; y++)
            for (int z = 0; z < 16; z++)
                for (int x = 0; x < 16; x++)
                    layer.set(x, y, z, value(spec, x, y, z));
        return layer;
    }

    /** The same queue-and-initialize order as loading a light-correct column from NBT. */
    private static void savedPadding() {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var obstructed = fixtureChunk(true);
        verifySavedPadding(obstructed, false);
        verifySavedPadding(obstructed, true);
        verifyVoidPadding(fixtureChunk(false));
        verifyUnsupportedPadIsNotEvidence();
    }

    private static LightChunk fixtureChunk(boolean obstructed) {
        int bottom = MIN_SECTION * 16;
        int top = (MIN_SECTION + SECTIONS - 1) * 16;
        LightChunk chunk = new LightChunk() {
            final ChunkSkyLightSources sources = new ChunkSkyLightSources(this);
            public net.minecraft.world.level.block.entity.BlockEntity getBlockEntity(BlockPos pos) { return null; }
            public BlockState getBlockState(BlockPos pos) {
                return (obstructed && pos.getX() == 0 && pos.getZ() == 0 && (pos.getY() == top || pos.getY() == bottom))
                    ? Blocks.STONE.defaultBlockState() : Blocks.AIR.defaultBlockState();
            }
            public net.minecraft.world.level.material.FluidState getFluidState(BlockPos pos) { return getBlockState(pos).getFluidState(); }
            public int getHeight() { return SECTIONS * 16; }
            public int getMinY() { return bottom; }
            public void findBlockLightSources(java.util.function.BiConsumer<BlockPos, BlockState> consumer) {}
            public ChunkSkyLightSources getSkyLightSources() { return sources; }
        };
        if (obstructed) chunk.getSkyLightSources().update(chunk, 0, top, 0);
        return chunk;
    }

    private static void verifySavedPadding(LightChunk chunk, boolean initializedProto) {
        var engine = engine(true, chunk);
        var center = new ChunkPos(0, 0);
        var lowest = SectionPos.of(center, MIN_SECTION);
        engine.queueSectionData(LightLayer.SKY, lowest, layer("P7"));
        engine.queueSectionData(LightLayer.BLOCK, lowest, layer("U0"));
        engine.runLightUpdates();
        // The adjacent proto initially has not reported any section status.
        var before = new ClientboundLightUpdatePacketData(center, engine, null, null);
        checkMask(before.emptyBlockYMask());
        if (initializedProto) {
            // INITIALIZE_LIGHT on the proto, without converting it to a full chunk.
            engine.updateSectionStatus(SectionPos.of(1, MIN_SECTION + 1, 0), false);
            engine.runLightUpdates();
            var onProto = new ClientboundLightUpdatePacketData(center, engine, null, null);
            checkMask(onProto.emptyBlockYMask(), 2, 3);
        }
        engine.updateSectionStatus(lowest, false);
        engine.updateSectionStatus(SectionPos.of(center, MIN_SECTION + SECTIONS - 1), false);
        engine.runLightUpdates();
        engine.setLightEnabled(center, true);
        engine.runLightUpdates();
        var initial = new ClientboundLightUpdatePacketData(center, engine, null, null);
        checkMask(initial.skyYMask(), 0, 1, SECTIONS + 1);
        checkMask(initial.blockYMask(), 1);
        checkMask(initial.emptySkyYMask(), initializedProto ? new int[] {2, 3, SECTIONS - 1, SECTIONS}
            : new int[] {2, SECTIONS - 1, SECTIONS});
        checkMask(initial.emptyBlockYMask(), initializedProto ? new int[] {0, 2, 3, SECTIONS - 1, SECTIONS, SECTIONS + 1}
            : new int[] {0, 2, SECTIONS - 1, SECTIONS, SECTIONS + 1});
        byte[] row = Arrays.copyOf(layer("P7").getData(), 128);
        byte[] lower = initial.skyUpdates().getFirst();
        for (int y = 0; y < 16; y++) {
            if (!Arrays.equals(lower, y * 128, (y + 1) * 128, row, 0, 128)) {
                throw new AssertionError("saved lower pad must repeat the bottom row");
            }
        }
        if (!Arrays.equals(initial.skyUpdates().getLast(), new DataLayer(15).getData())
            || !Arrays.equals(initial.blockUpdates().getFirst(), new byte[2048])) {
            throw new AssertionError("saved upper pad must carry full sky; saved zero array must carry bytes");
        }
    }

    private static void verifyVoidPadding(LightChunk chunk) {
        var center = new ChunkPos(0, 0);
        var neighbour = SectionPos.of(1, MIN_SECTION, 0);
        var pad = new BlockPos(0, MIN_SECTION * 16 - 1, 0);
        for (boolean enabledBeforeSupport : new boolean[] {false, true}) {
            var engine = engine(true, chunk);
            if (enabledBeforeSupport) engine.setLightEnabled(center, true);
            engine.updateSectionStatus(neighbour, false);
            engine.runLightUpdates();
            if (!enabledBeforeSupport) engine.setLightEnabled(center, true);
            engine.runLightUpdates();
            var packet = new ClientboundLightUpdatePacketData(center, engine, null, null);
            if (enabledBeforeSupport) {
                checkMask(packet.skyYMask(), 0, 1, 2);
                checkMask(packet.emptySkyYMask());
                if (!Arrays.equals(packet.skyUpdates().getFirst(), new DataLayer(15).getData())) {
                    throw new AssertionError("pad established after enabling carries full sky");
                }
            } else {
                checkMask(packet.skyYMask());
                checkMask(packet.emptySkyYMask(), 0, 1, 2);
            }
            if (engine.getLayerListener(LightLayer.SKY).getLightValue(pad) != (enabledBeforeSupport ? 15 : 0)) {
                throw new AssertionError("void pad sky does not agree with the allocated layer");
            }
            engine.updateSectionStatus(neighbour, true);
            engine.runLightUpdates();
            var resend = new ClientboundLightUpdatePacketData(center, engine, null, null);
            checkMask(resend.skyYMask());
            checkMask(resend.emptySkyYMask());
            checkMask(resend.emptyBlockYMask());
            if (engine.getLayerListener(LightLayer.SKY).getLightValue(pad) != 15) {
                throw new AssertionError("removed void pad must again read as open sky");
            }
            engine.updateSectionStatus(neighbour, false);
            engine.runLightUpdates();
            var restored = new ClientboundLightUpdatePacketData(center, engine, null, null);
            checkMask(restored.skyYMask(), 0, 1, 2);
            checkMask(restored.emptySkyYMask());
            if (!Arrays.equals(restored.skyUpdates().getFirst(), new DataLayer(15).getData())) {
                throw new AssertionError("returning support must create a new full-sky layer");
            }
        }
    }

    private static void verifyUnsupportedPadIsNotEvidence() {
        var engine = engine(true, null);
        var center = new ChunkPos(0, 0);
        var pad = SectionPos.of(center, MIN_SECTION - 1);
        engine.queueSectionData(LightLayer.SKY, pad, new DataLayer(new byte[2048]));
        // The section one above the lowest terrain section supports that lowest section,
        // but not the pad. A read below the column can still inherit its zero bottom row.
        engine.updateSectionStatus(SectionPos.of(center, MIN_SECTION + 1), false);
        engine.runLightUpdates();
        var packet = new ClientboundLightUpdatePacketData(center, engine, null, null);
        checkMask(packet.skyYMask(), 0);
        if (!Arrays.equals(packet.skyUpdates().getFirst(), new byte[2048])
            || engine.getLayerListener(LightLayer.SKY).getLightValue(pad.origin()) != 0
            || !visibleLowerSkyPad(engine, center, "packed-zero").equals("unsupported")) {
            throw new AssertionError("an explicit queued zero pad is not a supported dark pad");
        }
    }

    static String visibleLowerSkyPad(LevelLightEngine engine, ChunkPos column, String received) {
        if (!received.equals("empty") && !received.equals("packed-zero")) return received;
        var pad = SectionPos.of(column, engine.getMinLightSection());
        // getDataLayerData can see the queued copy even when no visible layer exists.
        if (engine.getDebugSectionType(LightLayer.SKY, pad) != SectionType.LIGHT_ONLY) return "unsupported";
        var sky = engine.getLayerListener(LightLayer.SKY);
        var origin = pad.origin();
        var pos = new BlockPos.MutableBlockPos();
        for (int i = 0; i < 4096; i++) {
            pos.set(origin.getX() + (i & 15), origin.getY() + (i >> 8), origin.getZ() + (i >> 4 & 15));
            if (sky.getLightValue(pos) != 0) return "not-dark";
        }
        return received;
    }

    private static void writeOrderedPadding(Path output) throws Exception {
        ParityData.init();
        var factory = PalettedContainerFactory.create(ParityData.access);
        var center = new ChunkPos(0, 0);
        var lowest = SectionPos.of(center, MIN_SECTION);
        var pad = SectionPos.of(center, MIN_SECTION - 1);
        var upperNeighbour = SectionPos.of(1, MIN_SECTION + 1, 0);
        var lowerNeighbour = SectionPos.of(0, MIN_SECTION, 1);
        var hex = HexFormat.of();
        var out = new ArrayList<String>();
        for (boolean arrayFirst : new boolean[] {false, true}) {
            var engine = engine(true, null);
            // Open the source section in both orders, but only the lower neighbour opens the pad.
            engine.updateSectionStatus(upperNeighbour, false);
            engine.runLightUpdates();
            if (!arrayFirst) {
                engine.updateSectionStatus(lowerNeighbour, false);
                engine.runLightUpdates();
            }
            var sky = new DataLayer();
            sky.set(0, 1, 0, 15); // A packed layer with a zero first row.
            engine.queueSectionData(LightLayer.SKY, lowest, sky);
            engine.runLightUpdates();
            if (arrayFirst) {
                engine.updateSectionStatus(lowerNeighbour, false);
                engine.runLightUpdates();
            }
            var skyListener = engine.getLayerListener(LightLayer.SKY);
            var lower = skyListener.getDataLayerData(pad);
            if (lower == null || lower.isEmpty() == arrayFirst
                || !Arrays.equals(lower.copy().getData(), new byte[2048])) {
                throw new AssertionError("the dark pad must retain its source's representation at allocation");
            }
            out.add("== " + (arrayFirst ? "array-before-status" : "status-before-array"));
            out.add("storage " + (lower.isEmpty() ? "empty" : "packed"));

            // This oracle has no ServerLevel/save thread. Select the light layers as in
            // SerializableChunkData.copyOf, then use the actual NBT writer for those sections.
            var sections = new ArrayList<SerializableChunkData.SectionData>();
            var blockListener = engine.getLayerListener(LightLayer.BLOCK);
            for (int y = engine.getMinLightSection(); y < engine.getMaxLightSection(); y++) {
                var at = SectionPos.of(center, y);
                var s = skyListener.getDataLayerData(at);
                var b = blockListener.getDataLayerData(at);
                sections.add(new SerializableChunkData.SectionData(y, null,
                    b == null || b.isEmpty() ? null : b.copy(), s == null || s.isEmpty() ? null : s.copy()));
            }
            var saved = new SerializableChunkData(factory, center, MIN_SECTION, 0, 0, ChunkStatus.FULL,
                null, null, UpgradeData.EMPTY, Map.of(), new ChunkAccess.PackedTicks(List.of(), List.of()),
                new ShortList[SECTIONS], true, sections, List.of(), List.of(), new CompoundTag()).write();
            for (var entry : saved.getListOrEmpty("sections")) {
                var section = (CompoundTag) entry;
                int y = section.getByte("Y").orElseThrow();
                for (String key : List.of("SkyLight", "BlockLight")) {
                    section.getByteArray(key).ifPresent(bytes -> out.add("save " + y + " " + key + " " + hex.formatHex(bytes)));
                }
            }

            var packet = new ClientboundLightUpdatePacketData(center, engine, null, null);
            var buffer = Unpooled.buffer();
            try {
                ClientboundLightUpdatePacketData.STREAM_CODEC.encode(buffer, packet);
                byte[] bytes = new byte[buffer.readableBytes()];
                buffer.readBytes(bytes);
                out.add("packet " + hex.formatHex(bytes));
            } finally {
                buffer.release();
            }
        }
        Files.write(output, out);
    }

    private static void checkMask(BitSet mask, int... bits) {
        var expected = new BitSet();
        for (int bit : bits) expected.set(bit);
        if (!mask.equals(expected)) throw new AssertionError("expected " + expected + ", got " + mask);
    }

    public static void main(String[] args) throws Exception {
        List<String> out = new ArrayList<>();
        LevelLightEngine engine = null;
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank() || line.startsWith("#")) continue;
            String[] f = line.split(" ");
            switch (f[0]) {
                case "case" -> {
                    engine = engine(f.length < 3 || !f[2].equals("nosky"), null);
                    out.add("== " + f[1]);
                }
                case "column" -> {
                    int cx = Integer.parseInt(f[1]), cz = Integer.parseInt(f[2]);
                    List<Integer> nonAir = new ArrayList<>();
                    for (String s : f[3].substring("nonair=".length()).split(","))
                        if (!s.isEmpty()) nonAir.add(Integer.parseInt(s));
                    ChunkPos pos = new ChunkPos(cx, cz);
                    for (int y = MIN_SECTION; y < MIN_SECTION + SECTIONS; y++)
                        engine.updateSectionStatus(SectionPos.of(pos, y), !nonAir.contains(y));
                    for (String s : f[4].substring("stored=".length()).split(",")) {
                        String[] p = s.split(":");
                        String[] k = p[1].split("/");
                        SectionPos at = SectionPos.of(cx, Integer.parseInt(p[0]), cz);
                        engine.queueSectionData(LightLayer.SKY, at, layer(k[0]));
                        engine.queueSectionData(LightLayer.BLOCK, at, layer(k[1]));
                    }
                    engine.setLightEnabled(pos, true);
                    engine.runLightUpdates();
                }
                case "drop" -> {
                    ChunkPos pos = new ChunkPos(Integer.parseInt(f[1]), Integer.parseInt(f[2]));
                    engine.setLightEnabled(pos, false);
                    for (int y = MIN_SECTION - 1; y < MIN_SECTION + SECTIONS + 1; y++) {
                        engine.queueSectionData(LightLayer.BLOCK, SectionPos.of(pos, y), null);
                        engine.queueSectionData(LightLayer.SKY, SectionPos.of(pos, y), null);
                    }
                    for (int y = MIN_SECTION; y < MIN_SECTION + SECTIONS; y++)
                        engine.updateSectionStatus(SectionPos.of(pos, y), true);
                    engine.runLightUpdates();
                }
                case "read" -> {
                    BlockPos pos = new BlockPos(Integer.parseInt(f[1]), Integer.parseInt(f[2]), Integer.parseInt(f[3]));
                    out.add(line + " -> " + engine.getLayerListener(LightLayer.SKY).getLightValue(pos) + " "
                        + engine.getLayerListener(LightLayer.BLOCK).getLightValue(pos));
                }
                default -> throw new IllegalArgumentException(line);
            }
        }
        savedPadding();
        Files.write(Path.of(args[1]), out);
        writeOrderedPadding(Path.of(args[2]));
    }
}
