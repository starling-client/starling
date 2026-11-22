package dev.sunriselab.parity;

import java.io.BufferedWriter;
import java.io.IOException;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;

/**
 * JDK {@code String.hashCode} for tools/test_string_hash_parity.py: the same strings as the Rust
 * probe ({@code crates/sunrise-core/src/bin/string_hash_parity.rs}) from the same seeds, each a
 * fresh {@code String} (compact Latin-1 when every unit fits, as the JDK makes it), hashed once.
 * Prints a streaming digest per chunk, or dumps one chunk's hashes. Arguments as the probe's.
 */
public final class VanillaStringHashOracle {
    static final int EXHAUSTIVE_CHUNKS = 1 + 65_536;
    static final int MAX_LEN = 600;

    interface Each { void accept(char[] units, int len, int hash); }

    static final class SplitMix {
        long state;
        SplitMix(long seed, long chunk) { state = seed ^ (chunk * 0xD1B54A32D192ED03L); }
        long next() {
            state += 0x9E3779B97F4A7C15L;
            long z = state;
            z = (z ^ (z >>> 30)) * 0xBF58476D1CE4E5B9L;
            z = (z ^ (z >>> 27)) * 0x94D049BB133111EBL;
            return z ^ (z >>> 31);
        }
    }

    static int randomString(SplitMix rng, char[] out) {
        int len = (int) Long.remainderUnsigned(rng.next(), MAX_LEN + 1);
        int mode = (int) (rng.next() & 3);
        for (int i = 0; i < len; i++) {
            long r = rng.next();
            int b = (int) (r >>> 8) & 0xFFFF;
            int unit;
            if (mode == 0) unit = b & 0x7F;
            else if (mode == 1) unit = b & 0xFF;
            else if (mode == 2) unit = b;
            else switch ((int) (r & 7)) {
                case 0, 1, 2 -> unit = b & 0x7F;
                case 3 -> unit = b & 0xFF;
                case 4 -> unit = b;
                case 5 -> {
                    if (i + 1 < len) {
                        out[i++] = (char) (0xD800 | (b & 0x3FF));
                        unit = 0xDC00 | ((int) (r >>> 24) & 0x3FF);
                    } else unit = b & 0x7F;
                }
                case 6 -> unit = 0xD800 | (b & 0x3FF);
                default -> unit = 0xDC00 | (b & 0x3FF);
            }
            out[i] = (char) unit;
        }
        return len;
    }

    static void exhaustiveChunk(long chunk, Each each) {
        char[] s = new char[2];
        if (chunk == 0) {
            each.accept(s, 0, new String(s, 0, 0).hashCode());
            for (int a = 0; a <= 0xFFFF; a++) {
                s[0] = (char) a;
                each.accept(s, 1, new String(s, 0, 1).hashCode());
            }
        } else {
            s[0] = (char) (chunk - 1);
            for (int b = 0; b <= 0xFFFF; b++) {
                s[1] = (char) b;
                each.accept(s, 2, new String(s, 0, 2).hashCode());
            }
        }
    }

    static void randomChunk(long seed, long chunk, long perChunk, Each each) {
        SplitMix rng = new SplitMix(seed, chunk);
        char[] buffer = new char[MAX_LEN];
        for (long i = 0; i < perChunk; i++) {
            int len = randomString(rng, buffer);
            each.accept(buffer, len, new String(buffer, 0, len).hashCode());
        }
    }

    interface Run { void run(long chunk, Each each); }

    static void digests(String tag, long first, long chunks, int threads, Run run) throws Exception {
        ExecutorService pool = Executors.newFixedThreadPool(threads);
        List<Future<long[]>> results = new ArrayList<>();
        for (long c = first; c < first + chunks; c++) {
            long chunk = c;
            results.add(pool.submit(() -> {
                long[] d = {0, 0xCBF29CE484222325L};
                run.run(chunk, (units, len, hash) -> {
                    d[0]++;
                    d[1] = (d[1] ^ (hash & 0xFFFFFFFFL)) * 0x100000001B3L;
                });
                return d;
            }));
        }
        try (PrintWriter out = new PrintWriter(new BufferedWriter(new java.io.OutputStreamWriter(System.out)))) {
            for (int i = 0; i < results.size(); i++) {
                long[] d = results.get(i).get();
                out.printf("%s %d %d %016x%n", tag, first + i, d[0], d[1]);
            }
        }
        pool.shutdown();
    }

    static void dump(String file, java.util.function.Consumer<Each> fill) throws IOException {
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(file)))) {
            long[] index = {0};
            fill.accept((units, len, hash) -> {
                StringBuilder hex = new StringBuilder();
                for (int i = 0; i < len; i++) {
                    if (i > 0) hex.append(',');
                    hex.append(String.format("%04x", (int) units[i]));
                }
                out.println(index[0]++ + " " + hex + " " + hash);
            });
        }
    }

    public static void main(String[] a) throws Exception {
        switch (a[0]) {
            case "exhaustive" -> digests("E", 0, EXHAUSTIVE_CHUNKS, Integer.parseInt(a[1]), VanillaStringHashOracle::exhaustiveChunk);
            case "random" -> {
                long seed = Long.parseUnsignedLong(a[1]), perChunk = Long.parseLong(a[4]);
                digests("R", Long.parseLong(a[2]), Long.parseLong(a[3]), Integer.parseInt(a[5]),
                        (chunk, each) -> randomChunk(seed, chunk, perChunk, each));
            }
            case "dump" -> {
                if (a[1].equals("exhaustive")) dump(a[3], each -> exhaustiveChunk(Long.parseLong(a[2]), each));
                else dump(a[5], each -> randomChunk(Long.parseUnsignedLong(a[2]), Long.parseLong(a[3]), Long.parseLong(a[4]), each));
            }
            default -> throw new IllegalArgumentException("unknown mode " + a[0]);
        }
    }
}
