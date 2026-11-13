// Development oracle: run against the player's own pinned 26.3 game and libraries.
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import net.minecraft.nbt.*;
import java.nio.file.*;

public class JsonNbtOracle {
    static JsonObject describe(Tag tag) {
        JsonObject out = new JsonObject();
        out.addProperty("type", tag.getId());
        JsonElement value = switch (tag) {
            case CompoundTag compound -> {
                JsonObject fields = new JsonObject();
                compound.keySet().stream().sorted().forEach(key -> fields.add(key, describe(compound.get(key))));
                yield fields;
            }
            case ListTag list -> {
                JsonArray entries = new JsonArray();
                for (Tag entry : list) entries.add(describe(entry));
                yield entries;
            }
            case EndTag ignored -> JsonNull.INSTANCE;
            case FloatTag f -> new JsonPrimitive(Integer.toUnsignedString(Float.floatToIntBits(f.floatValue())));
            case DoubleTag d -> new JsonPrimitive(Long.toUnsignedString(Double.doubleToLongBits(d.doubleValue())));
            case StringTag s -> new JsonPrimitive(s.value());
            case NumericTag n -> new JsonPrimitive(n.longValue() + "");
            default -> throw new IllegalArgumentException("Unexpected array tag: " + tag);
        };
        out.add("value", value);
        return out;
    }
    public static void main(String[] args) throws Exception {
        JsonArray corpus = JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray();
        JsonArray results = new JsonArray();
        for (JsonElement input : corpus) {
            try {
                results.add(describe(JsonOps.INSTANCE.convertTo(NbtOps.INSTANCE, input)));
            } catch (NullPointerException expectedForNestedNull) {
                JsonObject error = new JsonObject();
                error.addProperty("error", "NestedNull");
                results.add(error);
            }
        }
        Files.writeString(Path.of(args[1]), results.toString());
    }
}
