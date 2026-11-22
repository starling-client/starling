package dev.sunriselab.parity;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeMap;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.crafting.AbstractCookingRecipe;
import net.minecraft.world.item.crafting.RecipeManager;
import net.minecraft.world.level.block.AbstractFurnaceBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.HopperBlock;
import net.minecraft.world.level.block.entity.AbstractFurnaceBlockEntity;
import net.minecraft.world.level.block.entity.HopperBlockEntity;
import net.minecraft.world.level.block.state.BlockState;

/**
 * Original oracle for the furnace block entity: a real `RecipeManager` and a
 * block-update level holding a furnace, blast furnace or smoker, ticked with
 * the real `AbstractFurnaceBlockEntity.serverTick`. Fixture lines are
 * "furnace KIND TICKS INPUT FUEL OUTPUT TIMER TOTAL LIT LITTOTAL SPEED
 * CHANGE_AT NEW_INPUT"; `-` is an empty stack and CHANGE_AT -1 means no change.
 * Output is the three slots, the five timers, the LIT state, the `RecipesUsed`
 * counts and each used recipe's experience.
 */
public final class VanillaFurnaceOracle {
    private static Map<String, Block> blocks;

    public static void main(String[] args) throws Exception {
        Path in = Path.of(args[0]), out = Path.of(args[1]);
        ParityData.init(in.resolveSibling("fixture-pack"));
        blocks = Map.of(
                "furnace", Blocks.FURNACE,
                "blast_furnace", Blocks.BLAST_FURNACE,
                "smoker", Blocks.SMOKER);
        VanillaBlockUpdateOracle.AIR = Blocks.AIR.defaultBlockState();
        var allocator = Class.forName("sun.misc.Unsafe");
        var unsafeField = allocator.getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        VanillaBlockUpdateOracle.unsafe = unsafeField.get(null);
        VanillaBlockUpdateOracle.allocate = allocator.getMethod("allocateInstance", Class.class);
        var recipes = new RecipeManager(ParityData.lookup);
        recipes.finalizeRecipeLoading(FeatureFlags.VANILLA_SET);
        VanillaBlockUpdateOracle.Server.recipes = recipes;
        Map<String, Float> experience = new TreeMap<>();
        for (var holder : recipes.getRecipes()) {
            if (holder.value() instanceof AbstractCookingRecipe cooking) {
                experience.put(holder.id().identifier().toString(), cooking.experience());
            }
        }
        try (var w = new PrintWriter(Files.newBufferedWriter(out, StandardCharsets.UTF_8));
             var lines = Files.newBufferedReader(in, StandardCharsets.UTF_8)) {
            int n = 0;
            for (String line = lines.readLine(); line != null; line = lines.readLine()) {
                if (line.startsWith("#hopper ")) checkSideHopper(Path.of(line.substring(8)));
                if (line.startsWith("#")) continue;
                w.println(run(n++, line, recipes, experience));
            }
        }
        System.exit(0);
    }

    private static Field field(Class<?> c, String name) throws Exception {
        Field f = c.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    private static String run(int n, String line, RecipeManager recipes, Map<String, Float> experience)
            throws Exception {
        String[] f = line.split(" ");
        Block block = blocks.get(f[1]);
        int ticks = Integer.parseInt(f[2]);
        ItemStack input = ParityData.stack(f[3]);
        ItemStack fuel = ParityData.stack(f[4]);
        ItemStack output = ParityData.stack(f[5]);
        int timer = Integer.parseInt(f[6]);
        int total = Integer.parseInt(f[7]);
        int lit = Integer.parseInt(f[8]);
        int litTotal = Integer.parseInt(f[9]);
        float speed = Float.parseFloat(f[10]);
        int changeAt = Integer.parseInt(f[11]);
        ItemStack newInput = ParityData.stack(f[12]);

        var pos = BlockPos.ZERO;
        var w = world(Map.of(pos, block.defaultBlockState()));
        BlockState state = w.getBlockState(pos);
        var be = (AbstractFurnaceBlockEntity) w.getBlockEntity(pos);
        be.setItem(0, input);
        be.setItem(1, fuel);
        be.setItem(2, output);
        field(AbstractFurnaceBlockEntity.class, "cookingTimer").setInt(be, timer);
        field(AbstractFurnaceBlockEntity.class, "cookingTotalTime").setInt(be, total);
        field(AbstractFurnaceBlockEntity.class, "litTimeRemaining").setInt(be, lit);
        field(AbstractFurnaceBlockEntity.class, "litTotalTime").setInt(be, litTotal);
        field(AbstractFurnaceBlockEntity.class, "speedMultiplier").setFloat(be, speed);
        for (int t = 0; t < ticks; t++) {
            if (t == changeAt) be.setItem(0, newInput);
            state = w.getBlockState(pos);
            AbstractFurnaceBlockEntity.serverTick(w, pos, state, be);
        }
        if (changeAt == ticks) be.setItem(0, newInput);
        state = w.getBlockState(pos);
        StringBuilder sb = new StringBuilder(Integer.toString(n));
        sb.append(" kind=").append(f[1]);
        for (int i = 0; i < 3; i++) sb.append(" s").append(i).append('=').append(ParityData.canon(be.getItem(i)));
        sb.append(" timer=").append(field(AbstractFurnaceBlockEntity.class, "cookingTimer").getInt(be))
          .append(" total=").append(field(AbstractFurnaceBlockEntity.class, "cookingTotalTime").getInt(be))
          .append(" lit=").append(field(AbstractFurnaceBlockEntity.class, "litTimeRemaining").getInt(be))
          .append(" littotal=").append(field(AbstractFurnaceBlockEntity.class, "litTotalTime").getInt(be))
          .append(" speed=").append(String.format(Locale.ROOT, "%08x", Float.floatToRawIntBits(
                  field(AbstractFurnaceBlockEntity.class, "speedMultiplier").getFloat(be))))
          .append(" state=").append(state.getValue(AbstractFurnaceBlock.LIT));
        @SuppressWarnings("unchecked")
        Map<Object, Integer> used = (Map<Object, Integer>) field(AbstractFurnaceBlockEntity.class, "recipesUsed").get(be);
        List<String> ids = new ArrayList<>();
        for (Object key : used.keySet()) ids.add(key.toString().replaceAll(".* / ", "").replaceAll("\\]$", ""));
        ids.sort(Comparator.naturalOrder());
        sb.append(" used=");
        for (int i = 0; i < ids.size(); i++) {
            if (i > 0) sb.append(',');
            String id = ids.get(i);
            sb.append(id).append('=').append(used.get(keyFor(used, id)));
        }
        sb.append(" xp=");
        for (int i = 0; i < ids.size(); i++) {
            if (i > 0) sb.append(',');
            String id = ids.get(i);
            sb.append(id).append('=').append(String.format(Locale.ROOT, "%08x",
                    Float.floatToRawIntBits(experience.getOrDefault(id, 0.0f))));
        }
        if (f.length > 14 && f[14].equals("rng")) {
            sb.append(" rng=").append(w.getRandom().nextInt());
        }
        return sb.toString();
    }

    private static VanillaBlockUpdateOracle.World world(Map<BlockPos, BlockState> blocks) throws Exception {
        JsonArray cells = new JsonArray();
        for (var entry : blocks.entrySet()) {
            JsonArray c = new JsonArray();
            c.add(entry.getKey().getX());
            c.add(entry.getKey().getY());
            c.add(entry.getKey().getZ());
            c.add(Block.getId(entry.getValue()));
            cells.add(c);
        }
        JsonObject fixture = new JsonObject();
        fixture.add("cells", cells);
        fixture.addProperty("seed", 0);
        var world = VanillaBlockUpdateOracle.build(fixture);
        world.registries = ParityData.access;
        return world;
    }

    /** The same fixture the Rust game daily drives through its hopper tick. */
    private static void checkSideHopper(Path fixture) throws Exception {
        BlockPos from = BlockPos.ZERO, to = from.east();
        BlockState hopperState = Blocks.HOPPER.defaultBlockState().setValue(HopperBlock.FACING, Direction.EAST);
        for (String line : Files.readAllLines(fixture)) {
            if (line.startsWith("#")) continue;
            String[] fields = line.split("\t");
            ItemStack offered = ParityData.stack(fields[1]);
            ItemStack held = ParityData.stack(fields[2]);
            boolean moves = Boolean.parseBoolean(fields[3]);
            var world = world(Map.of(from, hopperState, to, Blocks.FURNACE.defaultBlockState()));
            var hopper = (HopperBlockEntity) world.getBlockEntity(from);
            var furnace = (AbstractFurnaceBlockEntity) world.getBlockEntity(to);
            hopper.setItem(0, offered.copy());
            furnace.setItem(1, held.copy());
            HopperBlockEntity.pushItemsTick(world, from, hopperState, hopper);
            int transferred = moves ? 1 : 0;
            ItemStack source = offered.copyWithCount(offered.getCount() - transferred);
            ItemStack destination = held.isEmpty() && moves
                    ? offered.copyWithCount(1) : held.copyWithCount(held.getCount() + transferred);
            if (!ItemStack.matches(source, hopper.getItem(0)) || !ItemStack.matches(destination, furnace.getItem(1))
                    || !furnace.getItem(0).isEmpty() || !furnace.getItem(2).isEmpty()) {
                throw new AssertionError("Unexpected side-hopper transfer: " + fields[0]);
            }
        }
    }

    private static Object keyFor(Map<Object, Integer> used, String id) {
        for (Object key : used.keySet()) {
            if (key.toString().replaceAll(".* / ", "").replaceAll("\\]$", "").equals(id)) return key;
        }
        return null;
    }
}
