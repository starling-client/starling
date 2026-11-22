package dev.sunriselab.parity;

import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.multiplayer.SessionSearchTrees;
import net.minecraft.client.resources.language.ClientLanguage;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.locale.Language;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.server.packs.PackLocationInfo;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.PathPackResources;
import net.minecraft.server.packs.repository.PackSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.InstrumentTags;
import net.minecraft.tags.PaintingVariantTags;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.CreativeModeTab;
import net.minecraft.world.item.CreativeModeTabs;
import net.minecraft.world.item.ItemStack;

/**
 * Original oracle for the creative inventory (test_creative_parity.py): writes
 * the exported creative-inventory.json Sunrise loads, then what the vanilla
 * client shows: every tab's display items (canonical stacks) and, for each
 * query, CreativeModeInventoryScreen.refreshSearchResults' items as indices
 * into the search tab, searched through the client's own SessionSearchTrees
 * (FullTextSearchTree over creative tooltip lines and item ids, IdSearchTree
 * over item tags after "#").
 *
 *   VanillaCreativeSearchOracle QUERIES.json EXPORT.json OUT.json RESOURCES [permissions]
 *
 * With "permissions", the screen's tabs are built as for an operator in creative
 * with the Operator Items Tab option on (CreativeModeInventoryScreen.hasPermissions).
 *
 * The queries are the file's list, then every search stack's item path and
 * tooltip lines and every item tag, each also as a prefix and with "#".
 * Then the same search trees are reloaded in French for the focused queries.
 */
public final class VanillaCreativeSearchOracle {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var lookup = ParityData.lookup;
        boolean hasPermissions = args.length > 4 && args[4].equals("permissions");
        Files.writeString(Path.of(args[1]),
            VanillaCreativeTabsExporter.export(FeatureFlags.DEFAULT_FLAGS, lookup).toString(),
            StandardCharsets.UTF_8);
        // CreativeModeInventoryScreen.tryRebuildTabContents, as the screen calls it.
        CreativeModeTabs.tryRebuildTabContents(FeatureFlags.DEFAULT_FLAGS, hasPermissions, lookup);
        List<ItemStack> searchItems = List.copyOf(CreativeModeTabs.searchTab().getDisplayItems());
        var trees = new SessionSearchTrees();
        trees.updateCreativeTooltips(lookup, searchItems);
        trees.updateCreativeTags(searchItems);

        var out = new JsonObject();
        var tabs = new JsonArray();
        for (CreativeModeTab tab : BuiltInRegistries.CREATIVE_MODE_TAB) {
            var o = new JsonObject();
            o.addProperty("id", BuiltInRegistries.CREATIVE_MODE_TAB.getKey(tab).toString());
            o.addProperty("type", tab.getType().name());
            o.addProperty("shown", tab.shouldDisplay());
            o.addProperty("title", tab.getDisplayName().getString());
            o.addProperty("icon", BuiltInRegistries.ITEM.getKey(tab.getIconItem().getItem()).toString());
            o.addProperty("row", tab.row().name());
            o.addProperty("column", tab.column());
            var items = new JsonArray();
            for (ItemStack stack : tab.getDisplayItems()) items.add(ParityData.canon(stack));
            o.add("items", items);
            var search = new JsonArray();
            for (ItemStack stack : tab.getSearchTabDisplayItems()) search.add(ParityData.canon(stack));
            o.add("search_items", search);
            tabs.add(o);
        }
        out.add("tabs", tabs);
        out.add("holders", holderFacts(lookup));
        var features = new JsonArray();
        FeatureFlags.REGISTRY.toNames(FeatureFlags.DEFAULT_FLAGS).forEach(flag -> features.add(flag.toString()));
        out.add("features", features);

        var querySets = JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonObject();
        var queries = new LinkedHashSet<String>();
        for (var q : querySets.getAsJsonArray("english")) queries.add(q.getAsString());
        var context = net.minecraft.world.item.Item.TooltipContext.of(lookup);
        var flag = net.minecraft.world.item.TooltipFlag.Default.NORMAL.asCreative();
        var tags = new TreeSet<String>();
        for (ItemStack stack : searchItems) {
            String path = BuiltInRegistries.ITEM.getKey(stack.getItem()).getPath();
            queries.add(path);
            queries.add(path.substring(0, (path.length() + 1) / 2));
            for (String line : VanillaCreativeTabsExporter.tooltipLines(stack, context, flag)) {
                queries.add(line);
                queries.add(line.substring(0, (line.length() + 1) / 2));
            }
            stack.tags().forEach(t -> tags.add(t.location().toString()));
        }
        for (String tag : tags) {
            queries.add("#" + tag);
            queries.add("#" + tag.substring(tag.indexOf(':') + 1));
        }
        var index = new IdentityHashMap<ItemStack, Integer>();
        for (int i = 0; i < searchItems.size(); i++) index.put(searchItems.get(i), i);
        out.add("searches", queryResults(trees, index, queries));

        var pack = new PathPackResources(new PackLocationInfo("vanilla", Component.literal("vanilla"),
            PackSource.BUILT_IN, Optional.empty()), Path.of(args[3]));
        try (var resources = new MultiPackResourceManager(PackType.CLIENT_RESOURCES, List.of(pack))) {
            Language.inject(ClientLanguage.loadFrom(resources, List.of("en_us", "fr_fr"), false));
            trees.rebuildAfterLanguageChange();
            var french = new ArrayList<String>();
            for (var q : querySets.getAsJsonArray("french")) french.add(q.getAsString());
            out.add("frenchSearches", queryResults(trees, index, french));
        }
        Files.writeString(Path.of(args[2]), out.toString(), StandardCharsets.UTF_8);
    }

    /** Generator inputs in holder and tag order, separate from the expected lists above. */
    private static JsonObject holderFacts(HolderLookup.Provider lookup) {
        var holders = new JsonObject();
        var ops = lookup.createSerializationContext(JsonOps.INSTANCE);
        var potions = new JsonArray();
        lookup.lookupOrThrow(Registries.POTION).listElements().forEach(h -> potions.add(h.key().identifier().toString()));
        holders.add("potions", potions);
        var enchantments = new JsonArray();
        lookup.lookupOrThrow(Registries.ENCHANTMENT).listElements().forEach(h -> {
            var row = new JsonObject();
            row.addProperty("id", h.key().identifier().toString());
            row.addProperty("max_level", h.value().getMaxLevel());
            row.add("description", ComponentSerialization.CODEC.encodeStart(ops, h.value().description()).getOrThrow());
            enchantments.add(row);
        });
        holders.add("enchantments", enchantments);
        var paintings = new JsonArray();
        lookup.lookupOrThrow(Registries.PAINTING_VARIANT).listElements().forEach(h -> {
            var row = new JsonObject();
            row.addProperty("id", h.key().identifier().toString());
            row.addProperty("width", h.value().width());
            row.addProperty("height", h.value().height());
            row.addProperty("placeable", h.is(PaintingVariantTags.PLACEABLE));
            h.value().title().ifPresent(title -> row.add("title", ComponentSerialization.CODEC.encodeStart(ops, title).getOrThrow()));
            h.value().author().ifPresent(author -> row.add("author", ComponentSerialization.CODEC.encodeStart(ops, author).getOrThrow()));
            paintings.add(row);
        });
        holders.add("paintings", paintings);
        var horns = new JsonArray();
        lookup.lookupOrThrow(Registries.INSTRUMENT).get(InstrumentTags.GOAT_HORNS).orElseThrow()
            .forEach(h -> horns.add(h.unwrapKey().orElseThrow().identifier().toString()));
        holders.add("horns", horns);
        var instruments = new JsonObject();
        lookup.lookupOrThrow(Registries.INSTRUMENT).listElements().forEach(h -> instruments.add(
            h.key().identifier().toString(), ComponentSerialization.CODEC.encodeStart(ops, h.value().description()).getOrThrow()));
        holders.add("instrument_descriptions", instruments);
        var patterns = new JsonArray();
        lookup.lookupOrThrow(Registries.BANNER_PATTERN).listElements().forEach(h -> patterns.add(h.key().identifier().toString()));
        holders.add("banner_patterns", patterns);
        return holders;
    }

    private static JsonArray queryResults(SessionSearchTrees trees, Map<ItemStack, Integer> index,
                                          Iterable<String> queries) {
        var results = new JsonArray();
        for (String query : queries) {
            var r = new JsonObject();
            r.addProperty("query", query);
            var found = new JsonArray();
            for (ItemStack stack : refreshSearchResults(trees, query)) found.add(index.get(stack));
            r.add("results", found);
            results.add(r);
        }
        return results;
    }

    /** CreativeModeInventoryScreen.refreshSearchResults' items for the search box's value. */
    static List<ItemStack> refreshSearchResults(SessionSearchTrees trees, String searchTerm) {
        if (searchTerm.isEmpty()) return List.copyOf(CreativeModeTabs.searchTab().getDisplayItems());
        if (searchTerm.startsWith("#")) return trees.creativeTagSearch().search(searchTerm.substring(1).toLowerCase(Locale.ROOT));
        return trees.creativeNameSearch().search(searchTerm.toLowerCase(Locale.ROOT));
    }
}
