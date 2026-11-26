package dev.sunriselab.parity;

import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonNull;
import com.google.gson.JsonObject;
import com.mojang.authlib.GameProfile;
import com.mojang.authlib.minecraft.UserApiService;
import io.netty.buffer.Unpooled;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import java.util.Objects;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import net.minecraft.client.InputType;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.screens.inventory.BookEditScreen;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.client.input.MouseButtonInfo;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.component.DataComponents;
import net.minecraft.nbt.NbtOps;
import net.minecraft.nbt.Tag;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.chat.CommonComponents;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ServerboundEditBookPacket;
import net.minecraft.server.network.Filterable;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.WritableBookContent;

/** Records vanilla 26.3 WritableBookContent and BookEditScreen Done results from the real game classes. */
public final class VanillaWritableBookOracle {
    private static Object unsafe;
    private static Method allocate;

    @SuppressWarnings("unchecked")
    private static <T> T alloc(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    private static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        VanillaPackScreenOracle.put(target, owner, name, value);
    }

    private static final class CapturingConnection extends ClientPacketListener {
        List<Packet<?>> sent;

        private CapturingConnection() {
            super(null, null, null);
        }

        @Override
        public void send(Packet<?> packet) {
            sent.add(packet);
        }
    }

    private record CodecCase(String name, String snbt) {}

    private record BookEditCase(String name, int slot, InteractionHand hand, String snbt) {}

    private static final List<CodecCase> CODEC_CASES = List.of(
        new CodecCase(
            "mixed_filtered",
            "{id:\"minecraft:writable_book\",count:1,components:{\"minecraft:writable_book_content\":{pages:[\"one\",{raw:\"two\",filtered:\"t*o\"}]}}}"
        ),
        new CodecCase(
            "unfiltered",
            "{id:\"minecraft:writable_book\",count:1,components:{\"minecraft:writable_book_content\":{pages:[\"one\",\"two\"]}}}"
        ),
        new CodecCase(
            "different_filtered",
            "{id:\"minecraft:writable_book\",count:1,components:{\"minecraft:writable_book_content\":{pages:[\"one\",{raw:\"two\",filtered:\"***\"}]}}}"
        ),
        new CodecCase(
            "first_filtered",
            "{id:\"minecraft:writable_book\",count:1,components:{\"minecraft:writable_book_content\":{pages:[{raw:\"one\",filtered:\"o*e\"},\"two\"]}}}"
        ),
        new CodecCase(
            "all_filtered",
            "{id:\"minecraft:writable_book\",count:1,components:{\"minecraft:writable_book_content\":{pages:[{raw:\"one\",filtered:\"o*e\"},{raw:\"two\",filtered:\"t*o\"}]}}}"
        ),
        new CodecCase(
            "unfiltered_compound",
            "{id:\"minecraft:writable_book\",count:1,components:{\"minecraft:writable_book_content\":{pages:[{raw:\"one\"},{raw:\"two\"}]}}}"
        ),
        new CodecCase(
            "empty_omitted",
            "{id:\"minecraft:writable_book\",count:1,components:{\"minecraft:writable_book_content\":{}}}"
        )
    );

    private static final List<BookEditCase> BOOK_EDIT_CASES = List.of(
        new BookEditCase(
            "unedited_filtered_slot_0",
            0,
            InteractionHand.MAIN_HAND,
            "{id:\"minecraft:writable_book\",count:1,components:{\"minecraft:writable_book_content\":{pages:[\"one\",{raw:\"two\",filtered:\"t*o\"}]}}}"
        ),
        new BookEditCase(
            "unedited_filtered_slot_3",
            3,
            InteractionHand.MAIN_HAND,
            "{id:\"minecraft:writable_book\",count:1,components:{\"minecraft:writable_book_content\":{pages:[{raw:\"alpha\",filtered:\"a***a\"},\"beta\"]}}}"
        ),
        new BookEditCase(
            "unedited_trailing_empty_filtered",
            0,
            InteractionHand.MAIN_HAND,
            "{id:\"minecraft:writable_book\",count:1,components:{\"minecraft:writable_book_content\":{pages:[\"one\",{raw:\"\",filtered:\"*\"}]}}}"
        )
    );

    private static JsonArray rawPages(WritableBookContent content) {
        JsonArray out = new JsonArray();
        for (Filterable<String> page : content.pages()) {
            out.add(page.raw());
        }
        return out;
    }

    private static JsonArray filteredPages(WritableBookContent content) {
        JsonArray out = new JsonArray();
        for (Filterable<String> page : content.pages()) {
            if (page.filtered().isPresent()) {
                out.add(page.filtered().get());
            } else {
                out.add(JsonNull.INSTANCE);
            }
        }
        return out;
    }

    private static String encodeWire(ItemStack stack) {
        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), ParityData.access);
        ItemStack.OPTIONAL_STREAM_CODEC.encode(buf, stack);
        byte[] bytes = new byte[buf.readableBytes()];
        buf.readBytes(bytes);
        return HexFormat.of().formatHex(bytes);
    }

    private static ItemStack decodeWire(String hex) {
        byte[] bytes = HexFormat.of().parseHex(hex);
        var buf = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(bytes), ParityData.access);
        ItemStack decoded = ItemStack.OPTIONAL_STREAM_CODEC.decode(buf);
        if (buf.readableBytes() != 0) {
            throw new IllegalStateException("unread wire bytes: " + buf.readableBytes());
        }
        return decoded;
    }

    private static String encodeEditBookPacket(ServerboundEditBookPacket packet) {
        var buf = new FriendlyByteBuf(Unpooled.buffer());
        ServerboundEditBookPacket.STREAM_CODEC.encode(buf, packet);
        byte[] bytes = new byte[buf.readableBytes()];
        buf.readBytes(bytes);
        return HexFormat.of().formatHex(bytes);
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        Field unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        VanillaPackScreenOracle.unsafe = unsafe;
        VanillaPackScreenOracle.allocate = allocate;

        JsonObject root = new JsonObject();
        root.addProperty("minecraft", "26.3");
        root.addProperty("oracle", "VanillaWritableBookOracle");

        List<ItemStack> parsedStacks = new ArrayList<>();
        List<WritableBookContent> parsedContents = new ArrayList<>();
        JsonArray codecArray = new JsonArray();
        for (CodecCase c : CODEC_CASES) {
            ItemStack stack = ParityData.stack(c.snbt());
            WritableBookContent content = stack.get(DataComponents.WRITABLE_BOOK_CONTENT);
            if (content == null) {
                throw new IllegalStateException("missing WRITABLE_BOOK_CONTENT for " + c.name());
            }
            parsedStacks.add(stack);
            parsedContents.add(content);
            Tag encodedComponent = WritableBookContent.CODEC.encodeStart(NbtOps.INSTANCE, content).getOrThrow();
            WritableBookContent reloaded = WritableBookContent.CODEC.parse(NbtOps.INSTANCE, encodedComponent).getOrThrow();
            String wireHex = encodeWire(stack);
            ItemStack wireDecoded = decodeWire(wireHex);
            WritableBookContent wireContent = wireDecoded.get(DataComponents.WRITABLE_BOOK_CONTENT);

            JsonObject entry = new JsonObject();
            entry.addProperty("name", c.name());
            entry.addProperty("input_snbt", c.snbt());
            entry.addProperty("encoded_component_canon", ParityData.canon(encodedComponent));
            entry.addProperty("encoded_stack_canon", ParityData.canon(stack));
            entry.add("raw_pages", rawPages(reloaded));
            entry.add("filtered_pages", filteredPages(reloaded));
            entry.addProperty("reloaded_equal", Objects.equals(content, reloaded));
            entry.addProperty("wire_hex", wireHex);
            entry.addProperty("wire_roundtrip_equal", Objects.equals(content, wireContent));
            codecArray.add(entry);
        }
        root.add("codec_cases", codecArray);

        JsonArray equalityArray = new JsonArray();
        for (int i = 0; i < CODEC_CASES.size(); i++) {
            for (int j = i; j < CODEC_CASES.size(); j++) {
                JsonObject pair = new JsonObject();
                pair.addProperty("left", CODEC_CASES.get(i).name());
                pair.addProperty("right", CODEC_CASES.get(j).name());
                pair.addProperty("content_equal", Objects.equals(parsedContents.get(i), parsedContents.get(j)));
                pair.addProperty("stack_equal", ItemStack.isSameItemSameComponents(parsedStacks.get(i), parsedStacks.get(j)));
                equalityArray.add(pair);
            }
        }
        root.add("equality_cases", equalityArray);

        // Headless Minecraft + BookEditScreen harness for real 26.3 Done/saveChanges execution.
        var font = ParityFont.create(ParityFont.resources());
        VanillaPackScreenOracle.font = font;
        Minecraft mc = alloc(Minecraft.class);
        put(mc, Minecraft.class, "font", font);
        put(mc, Minecraft.class, "fontFilterFishy", font);
        put(mc, Minecraft.class, "lastInputType", InputType.MOUSE);
        put(mc, Minecraft.class, "textInputManager", new VanillaPackScreenOracle.NoTextInput());
        put(mc, Minecraft.class, "soundManager", alloc(VanillaPackScreenOracle.NoSound.class));
        put(mc, Minecraft.class, "keyboardHandler", alloc(VanillaPackScreenOracle.NoKeyboard.class));
        put(mc, Minecraft.class, "userPropertiesFuture", CompletableFuture.completedFuture(UserApiService.OFFLINE_PROPERTIES));
        var gui = alloc(VanillaPackScreenOracle.StubGui.class);
        gui.width = 640;
        gui.height = 360;
        put(mc, Minecraft.class, "gui", gui);
        CapturingConnection connection = alloc(CapturingConnection.class);
        connection.sent = new ArrayList<>();
        LocalPlayer player = alloc(LocalPlayer.class);
        put(player, LocalPlayer.class, "connection", connection);
        put(player, Player.class, "gameProfile", new GameProfile(UUID.fromString("00000000-0000-0000-0000-000000000001"), "Player"));
        Inventory inventory = alloc(Inventory.class);
        put(player, Player.class, "inventory", inventory);
        put(mc, Minecraft.class, "player", player);
        Field instance = Minecraft.class.getDeclaredField("instance");
        instance.setAccessible(true);
        instance.set(null, mc);

        JsonArray editArray = new JsonArray();
        for (BookEditCase c : BOOK_EDIT_CASES) {
            connection.sent.clear();
            gui.current = null;
            put(inventory, Inventory.class, "selected", c.slot());
            ItemStack book = ParityData.stack(c.snbt());
            WritableBookContent initialContent = book.getOrDefault(DataComponents.WRITABLE_BOOK_CONTENT, WritableBookContent.EMPTY);
            BookEditScreen screen = new BookEditScreen(player, book, c.hand(), initialContent);
            gui.setScreen(screen);

            Button doneButton = null;
            for (GuiEventListener child : screen.children()) {
                if (child instanceof Button button && button.getMessage().equals(CommonComponents.GUI_DONE)) {
                    doneButton = button;
                    break;
                }
            }
            if (doneButton == null) {
                throw new IllegalStateException("GUI_DONE button not found on BookEditScreen");
            }
            doneButton.onPress(new MouseButtonEvent(0.0, 0.0, new MouseButtonInfo(1, 0)));

            WritableBookContent afterContent = book.get(DataComponents.WRITABLE_BOOK_CONTENT);
            Tag afterComponentTag = afterContent == null
                ? null
                : WritableBookContent.CODEC.encodeStart(NbtOps.INSTANCE, afterContent).getOrThrow();

            JsonArray packetsJson = new JsonArray();
            for (Packet<?> pkt : connection.sent) {
                if (pkt instanceof ServerboundEditBookPacket edit) {
                    JsonObject po = new JsonObject();
                    po.addProperty("slot", edit.slot());
                    JsonArray pages = new JsonArray();
                    for (String p : edit.pages()) {
                        pages.add(p);
                    }
                    po.add("pages", pages);
                    if (edit.title().isPresent()) {
                        po.addProperty("title", edit.title().get());
                    } else {
                        po.add("title", JsonNull.INSTANCE);
                    }
                    po.addProperty("wire_hex", encodeEditBookPacket(edit));
                    packetsJson.add(po);
                } else {
                    throw new IllegalStateException("unexpected packet: " + pkt.getClass().getName());
                }
            }

            JsonObject entry = new JsonObject();
            entry.addProperty("name", c.name());
            entry.addProperty("slot", c.slot());
            entry.addProperty("hand", c.hand().name().toLowerCase(java.util.Locale.ROOT));
            entry.addProperty("text_filtering_enabled", mc.isTextFilteringEnabled());
            entry.addProperty("input_snbt", c.snbt());
            entry.addProperty("screen_closed", gui.current == null);
            entry.add("sent_edit_book_packets", packetsJson);
            entry.add("after_raw_pages", afterContent == null ? new JsonArray() : rawPages(afterContent));
            entry.add("after_filtered_pages", afterContent == null ? new JsonArray() : filteredPages(afterContent));
            entry.addProperty("after_component_canon", afterComponentTag == null ? "-" : ParityData.canon(afterComponentTag));
            entry.addProperty("after_stack_canon", ParityData.canon(book));
            editArray.add(entry);
        }
        root.add("book_edit_cases", editArray);

        String formatted = new GsonBuilder().setPrettyPrinting().serializeNulls().disableHtmlEscaping().create().toJson(root) + "\n";
        Path outPath = Path.of(args[0]);
        if (outPath.getParent() != null) {
            Files.createDirectories(outPath.getParent());
        }
        Files.writeString(outPath, formatted, StandardCharsets.UTF_8);
    }
}
