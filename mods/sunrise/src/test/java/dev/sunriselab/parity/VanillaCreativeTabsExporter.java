package dev.sunriselab.parity;

import com.google.gson.*;
import java.lang.invoke.MethodHandles;
import java.lang.reflect.Field;
import java.lang.reflect.Proxy;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.List;
import net.minecraft.ChatFormatting;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.chat.Component;
import net.minecraft.tags.PaintingVariantTags;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.CreativeModeTab;
import net.minecraft.world.item.CreativeModeTabs;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.TooltipFlag;

/**
 * Original exporter of the creative inventory: every tab's contents as vanilla's
 * own CreativeModeTabs.tryRebuildTabContents builds them over the vanilla data
 * pack's registries (category tabs first, then the search tab from their
 * search-tab items), each stack as SNBT with its components, and for every stack
 * of the search tab the inputs of the client's creative search trees
 * (SessionSearchTrees): its creative tooltip lines and its item tags.
 *
 * Build rows capture the generators before the feature filter. Runs backed by
 * dynamic holders are regenerated against the active world's registries.
 * Tooltip lines and tags are presentation metadata, not a source of stacks.
 *
 *   VanillaCreativeTabsExporter OUT.json
 */
public final class VanillaCreativeTabsExporter {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        Files.writeString(Path.of(args[0]), export(FeatureFlags.DEFAULT_FLAGS, ParityData.lookup).toString(),
            StandardCharsets.UTF_8);
    }

    public static JsonObject export(FeatureFlagSet features, HolderLookup.Provider lookup) throws ReflectiveOperationException {
        Field generator = CreativeModeTab.class.getDeclaredField("displayItemsGenerator");
        generator.setAccessible(true);
        CreativeModeTabs.tryRebuildTabContents(features, true, lookup);
        var tabs = new JsonArray();
        for (CreativeModeTab tab : BuiltInRegistries.CREATIVE_MODE_TAB) {
            var o = new JsonObject();
            var key = BuiltInRegistries.CREATIVE_MODE_TAB.getKey(tab);
            o.addProperty("id", key == null ? "" : key.toString());
            o.addProperty("type", tab.getType().name());
            o.addProperty("row", tab.row().name());
            o.addProperty("column", tab.column());
            o.addProperty("title", tab.getDisplayName().getString());
            o.addProperty("icon", snbt(tab.getIconItem()));
            var normalBuild = buildRows(generator, tab, lookup, false);
            var operatorBuild = buildRows(generator, tab, lookup, true);
            o.add("build", normalBuild);
            if (!normalBuild.equals(operatorBuild)) o.add("operator_build", operatorBuild);
            if (tab.getType() == CreativeModeTab.Type.SEARCH) {
                var context = Item.TooltipContext.of(lookup);
                var flag = TooltipFlag.Default.NORMAL.asCreative();
                var metadata = new JsonArray();
                for (ItemStack stack : tab.getDisplayItems()) {
                    var row = new JsonObject();
                    row.addProperty("stack", snbt(stack));
                    row.add("lines", strings(tooltipLines(stack, context, flag)));
                    row.add("tags", strings(stack.tags().map(t -> t.location().toString()).toList()));
                    metadata.add(row);
                }
                o.add("search_metadata", metadata);
            }
            tabs.add(o);
        }
        var out = new JsonObject();
        out.addProperty("schema", 4);
        out.add("tabs", tabs);
        var itemFeatures = new JsonObject();
        for (var item : BuiltInRegistries.ITEM) {
            itemFeatures.add(BuiltInRegistries.ITEM.getKey(item).toString(), featureIds(item.requiredFeatures()));
        }
        var potionFeatures = new JsonObject();
        for (var potion : BuiltInRegistries.POTION) {
            potionFeatures.add(BuiltInRegistries.POTION.getKey(potion).toString(), featureIds(potion.requiredFeatures()));
        }
        out.add("item_features", itemFeatures);
        out.add("potion_features", potionFeatures);
        return out;
    }

    private static JsonArray featureIds(FeatureFlagSet features) {
        return strings(FeatureFlags.REGISTRY.toNames(features).stream().map(Object::toString).sorted().toList());
    }

    /** Capture in vanilla generator order, not by diffing filtered or aggregated final tab lists. */
    private static JsonArray buildRows(Field field, CreativeModeTab tab, HolderLookup.Provider lookup, boolean operator)
        throws ReflectiveOperationException {
        var rows = new JsonArray();
        // The search generator depends on the other tabs in the same build, not this export.
        if (tab.getType() == CreativeModeTab.Type.SEARCH) return rows;
        // Output and TabVisibility are protected; use a proxy to record the one abstract accept
        // method, leaving all convenience overloads to vanilla's own default methods.
        var accept = CreativeModeTab.DisplayItemsGenerator.class.getDeclaredMethods()[0];
        var outputType = accept.getParameterTypes()[1];
        var access = MethodHandles.privateLookupIn(outputType, MethodHandles.lookup());
        var output = Proxy.newProxyInstance(outputType.getClassLoader(), new Class<?>[] { outputType }, (proxy, method, args) -> {
            if (method.isDefault()) return access.unreflectSpecial(method, outputType).bindTo(proxy).invokeWithArguments(args);
            var stack = (ItemStack) args[0];
            String visibility = ((Enum<?>) args[1]).name();
            var row = new JsonObject();
            row.addProperty("visibility", visibility);
            var painting = stack.get(DataComponents.PAINTING_VARIANT);
            var potion = stack.get(DataComponents.POTION_CONTENTS);
            var enchantments = stack.get(DataComponents.STORED_ENCHANTMENTS);
            var layers = stack.get(DataComponents.BANNER_PATTERNS);
            if (painting != null) {
                row.addProperty("kind", "paintings");
                row.addProperty("placeable", painting.is(PaintingVariantTags.PLACEABLE));
            } else if (potion != null && potion.potion().isPresent()) {
                row.addProperty("kind", "potions");
                row.addProperty("item", BuiltInRegistries.ITEM.getKey(stack.getItem()).toString());
            } else if (enchantments != null && !enchantments.isEmpty()) {
                row.addProperty("kind", "enchantments");
                row.addProperty("all_levels", visibility.equals("SEARCH_TAB_ONLY"));
            } else if (stack.has(DataComponents.INSTRUMENT)) {
                row.addProperty("kind", "horns");
            } else {
                row.addProperty("kind", layers != null && !layers.layers().isEmpty() ? "banner" : "stack");
                row.addProperty("snbt", snbt(stack));
            }
            // Consecutive emissions of a dynamic family describe one run at this location.
            // Static duplicate policy belongs to the builder, not the exporter.
            if (row.has("snbt") || rows.isEmpty() || !row.equals(rows.get(rows.size() - 1))) rows.add(row);
            return null;
        });
        var parameters = new CreativeModeTab.ItemDisplayParameters(FeatureFlags.REGISTRY.allFlags(), operator, lookup);
        accept.invoke(field.get(tab), parameters, output);
        return rows;
    }

    /** SessionSearchTrees.getTooltipLines for one stack. */
    public static List<String> tooltipLines(ItemStack stack, Item.TooltipContext context, TooltipFlag flag) {
        return stack.getTooltipLines(context, null, flag).stream()
            .map(Component::getString)
            .map(l -> ChatFormatting.stripFormatting(l).trim())
            .filter(s -> !s.isEmpty())
            .toList();
    }

    /** ItemStack.CODEC as SNBT (the form sunrise_client::item_data::parse_stack reads). */
    public static String snbt(ItemStack stack) {
        if (stack.isEmpty()) return "-";
        return ItemStack.CODEC.encodeStart(ParityData.ops, stack).getOrThrow().toString();
    }

    private static JsonArray strings(List<String> values) {
        var a = new JsonArray();
        values.forEach(a::add);
        return a;
    }
}
