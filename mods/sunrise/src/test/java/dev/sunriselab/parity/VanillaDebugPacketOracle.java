package dev.sunriselab.parity;

import io.netty.buffer.Unpooled;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.protocol.game.ClientboundDebugSamplePacket;
import net.minecraft.network.protocol.game.ServerboundDebugSubscriptionRequestPacket;
import net.minecraft.server.Bootstrap;
import net.minecraft.util.debug.DebugSubscriptions;
import net.minecraft.util.debugchart.RemoteDebugSampleType;

/** Debug sample / subscription request bodies through the native stream codecs. */
public final class VanillaDebugPacketOracle {
    static String hex(RegistryFriendlyByteBuf buf) {
        byte[] bytes = new byte[buf.readableBytes()]; buf.readBytes(bytes); buf.release();
        return HexFormat.of().formatHex(bytes);
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var access = RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY);
        var out = new ArrayList<String>();
        for (long[] sample : new long[][]{{}, {50_000_000L, -1L}, {0L, Long.MAX_VALUE, Long.MIN_VALUE, 1L << 40}}) {
            var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
            ClientboundDebugSamplePacket.STREAM_CODEC.encode(buf, new ClientboundDebugSamplePacket(sample, RemoteDebugSampleType.TICK_TIME));
            out.add("{\"packet\":\"sample\",\"sample\":" + Arrays.toString(sample) + ",\"kind\":" + RemoteDebugSampleType.TICK_TIME.ordinal() + ",\"wire\":\"" + hex(buf) + "\"}");
        }
        for (var set : List.of(Set.<net.minecraft.util.debug.DebugSubscription<?>>of(), Set.<net.minecraft.util.debug.DebugSubscription<?>>of(DebugSubscriptions.DEDICATED_SERVER_TICK_TIME))) {
            var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
            ServerboundDebugSubscriptionRequestPacket.STREAM_CODEC.encode(buf, new ServerboundDebugSubscriptionRequestPacket(set));
            var ids = new ArrayList<Integer>();
            for (var s : set) ids.add(BuiltInRegistries.DEBUG_SUBSCRIPTION.getId(s));
            out.add("{\"packet\":\"request\",\"ids\":" + ids + ",\"wire\":\"" + hex(buf) + "\"}");
        }
        out.add("{\"packet\":\"registry\",\"count\":" + BuiltInRegistries.DEBUG_SUBSCRIPTION.size() + "}");
        // Codec only: the request checks the registry and list size, and the sample checks
        // its enum. Oversized arrays fail later in the client handler, not while decoding.
        for (String body : new String[]{"00", "0100", "020000", "020100", "03010000", "010f", "0110", "01ffffffff0f", "ff",
                "20" + "00".repeat(32), "21" + "00".repeat(33)}) {
            var buf = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(HexFormat.of().parseHex(body)), access);
            String result;
            try {
                var ids = new TreeSet<Integer>();
                for (var s : ServerboundDebugSubscriptionRequestPacket.STREAM_CODEC.decode(buf).subscriptions()) ids.add(BuiltInRegistries.DEBUG_SUBSCRIPTION.getId(s));
                result = "ok " + ids;
            } catch (Throwable t) {
                result = "error";
            }
            out.add("{\"packet\":\"decode_request\",\"body\":\"" + body + "\",\"result\":\"" + result + "\"}");
        }
        String eight = "0000000000000007";
        for (String body : new String[]{"0000", "01" + eight + "00", "04" + eight.repeat(4) + "00", "05" + eight.repeat(5) + "00", "0001", "000f", "00ffffffff0f"}) {
            var buf = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(HexFormat.of().parseHex(body)), access);
            String result;
            try {
                var packet = ClientboundDebugSamplePacket.STREAM_CODEC.decode(buf);
                result = "ok " + Arrays.toString(packet.sample()) + " " + packet.debugSampleType().ordinal();
            } catch (Throwable t) {
                result = "error";
            }
            out.add("{\"packet\":\"decode_sample\",\"body\":\"" + body + "\",\"result\":\"" + result + "\"}");
        }
        Files.write(Path.of(args[0]), out);
    }
}
