package dev.sunriselab.parity;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.io.UTFDataFormatException;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Supplier;

/**
 * JDK side of tools/test_modified_utf8_parity.py: DataOutputStream.writeUTF and DataInputStream.readUTF on the inputs
 * that sunrise-core's src/bin/modified_utf8_parity.rs gives sunrise_core::modified_utf8, generated from the same
 * SplitMix64 seed. Prints one FNV-1a digest per chunk ("chunk records digest"), or with "dump MODE CHUNK" one line
 * per record. Usage: MODE FIRST COUNT THREADS | dump MODE CHUNK.
 */
public final class VanillaModifiedUtf8Oracle {
    static final long SEED = 0x53756e7269736555L;
    static final long RANDOM_CHUNK = 1L << 20;

    static final class SplitMix {
        long state;
        SplitMix(long mode, long chunk) {
            state = SEED ^ (mode * 0x9e3779b97f4a7c15L) ^ (chunk * 0xd1b54a32d192ed03L);
        }
        long next() {
            state += 0x9e3779b97f4a7c15L;
            long z = state;
            z = (z ^ (z >>> 30)) * 0xbf58476d1ce4e5b9L;
            z = (z ^ (z >>> 27)) * 0x94d049bb133111ebL;
            return z ^ (z >>> 31);
        }
    }

    static final byte[] ZERO = {0}, ONE = {1};

    /** A ByteArrayOutputStream whose bytes can be read without a copy. */
    static final class Output extends ByteArrayOutputStream {
        Output() { super(8192); }
        byte[] buffer() { return buf; }
    }

    /** A ByteArrayInputStream whose bytes can be replaced, so that one DataInputStream reads every record. */
    static final class Input extends ByteArrayInputStream {
        Input() { super(new byte[0]); }
        void set(byte[] bytes, int length) { buf = bytes; pos = 0; count = length; mark = 0; }
    }

    /** One chunk's state: the digest, or the dump's lines, and the streams the records go through. */
    static final class Sink {
        long hash = 0xcbf29ce484222325L;
        long records;
        final StringBuilder dump;
        final Output bytes = new Output();
        final DataOutputStream data = new DataOutputStream(bytes);
        final Input input = new Input();
        final DataInputStream in = new DataInputStream(input);
        byte[] frame = new byte[8192];
        final byte[] result = new byte[1 << 18];

        Sink(boolean dump) { this.dump = dump ? new StringBuilder() : null; }

        void record(Supplier<String> what, byte[] out, int length) {
            record(what, out, length, out, 0);
        }

        /** One record: its input (for the dump) and its result, the bytes of `head` and then those of `tail`. */
        void record(Supplier<String> what, byte[] head, int headLength, byte[] tail, int tailLength) {
            records++;
            long h = hash;
            for (int i = 0; i < headLength; i++) h = (h ^ (head[i] & 0xff)) * 0x100000001b3L;
            for (int i = 0; i < tailLength; i++) h = (h ^ (tail[i] & 0xff)) * 0x100000001b3L;
            hash = h;
            if (dump != null) {
                dump.append(records - 1).append(' ').append(what.get()).append(' ');
                for (int i = 0; i < headLength; i++) dump.append(String.format("%02x", head[i] & 0xff));
                for (int i = 0; i < tailLength; i++) dump.append(String.format("%02x", tail[i] & 0xff));
                dump.append('\n');
            }
        }

        /** Folds `n` input bytes into the digest before their record. */
        void fold(byte[] b, int n) {
            long h = hash;
            for (int i = 0; i < n; i++) h = (h ^ (b[i] & 0xff)) * 0x100000001b3L;
            hash = h;
        }

        /** writeUTF's result: 01, its length and its bytes; or 00 where it throws. */
        void encode(char[] units, int n) throws IOException {
            String s = new String(units, 0, n);
            bytes.reset();
            try {
                data.writeUTF(s);
                record(() -> hexUnits(units, n), ONE, 1, bytes.buffer(), bytes.size());
            } catch (UTFDataFormatException e) {
                record(() -> hexUnits(units, n), ZERO, 1, ZERO, 0);
            }
        }

        /** readUTF's result: 01, the unit count (4 bytes) and the units; or 00 where it throws. */
        void decode(byte[] input, int n) throws IOException {
            if (frame.length < n + 2) frame = new byte[n + 2];
            frame[0] = (byte) (n >>> 8);
            frame[1] = (byte) n;
            System.arraycopy(input, 0, frame, 2, n);
            this.input.set(frame, n + 2);
            int length;
            try {
                String s = in.readUTF();
                result[0] = 1;
                int count = s.length();
                result[1] = (byte) (count >>> 24); result[2] = (byte) (count >>> 16);
                result[3] = (byte) (count >>> 8); result[4] = (byte) count;
                for (int i = 0; i < count; i++) {
                    char c = s.charAt(i);
                    result[5 + 2 * i] = (byte) (c >>> 8);
                    result[6 + 2 * i] = (byte) c;
                }
                length = 5 + 2 * count;
            } catch (UTFDataFormatException e) {
                result[0] = 0;
                length = 1;
            }
            record(() -> hexBytes(input, n), result, length);
        }
    }

    static String hexUnits(char[] units, int n) {
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < n; i++) {
            if (i > 0) b.append(',');
            b.append(String.format("%04x", (int) units[i]));
        }
        return b.toString();
    }

    static String hexBytes(byte[] bytes, int n) {
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < n; i++) b.append(String.format("%02x", bytes[i] & 0xff));
        return b.toString();
    }

    /** Units in a growable array, as the probe's Vec. */
    static final class Units {
        char[] a = new char[4096];
        int n;
        void push(long u) {
            if (n == a.length) a = java.util.Arrays.copyOf(a, n * 2);
            a[n++] = (char) u;
        }
    }

    static final class Bytes {
        byte[] a = new byte[4096];
        int n;
        void push(long b) {
            if (n == a.length) a = java.util.Arrays.copyOf(a, n * 2);
            a[n++] = (byte) b;
        }
    }

    /** The probe's random_units. */
    static void randomUnits(SplitMix rng, Units units) {
        units.n = 0;
        long r = rng.next();
        int len = (int) ((r & 0xff) == 0 ? (r >>> 8) % 2048 : (r >>> 8) % 48);
        long profile = (r >>> 40) & 3;
        while (units.n < len) {
            r = rng.next();
            long cls, v;
            if (profile == 0) { cls = 0; v = r >>> 3; }
            else if (profile == 1 && (r & 15) != 0) { cls = 8; v = r >>> 4; }
            else if (profile == 3) { cls = 4 + (r & 3); v = r >>> 3; }
            else { cls = r & 7; v = r >>> 3; }
            switch ((int) cls) {
                case 0, 1 -> units.push(v % 128);
                case 2 -> units.push(0x80 + v % 0x780);
                case 3 -> {
                    long u = 0x800 + v % (0x10000 - 0x800 - 0x800);
                    units.push(u >= 0xd800 ? u + 0x800 : u);
                }
                case 4 -> units.push(0xd800 + v % 1024);
                case 5 -> units.push(0xdc00 + v % 1024);
                case 6 -> { units.push(0xd800 + v % 1024); units.push(0xdc00 + (v >>> 10) % 1024); }
                case 7 -> units.push(v);
                default -> units.push(1 + v % 127);
            }
        }
    }

    static long cont(long v) { return 0x80 | (v & 0x3f); }

    /** The probe's random_bytes. */
    static void randomBytes(SplitMix rng, Bytes bytes) {
        bytes.n = 0;
        long r = rng.next();
        int len = (int) ((r & 0xff) == 0 ? (r >>> 8) % 1024 : (r >>> 8) % 40);
        long profile = (r >>> 40) & 3;
        while (bytes.n < len) {
            r = rng.next();
            long v = r >>> 4;
            long cls;
            if (profile == 0) cls = 15;
            else if (profile == 1 && (r & 63) != 0) cls = 4 + v % 3;
            else if (profile == 2) cls = 4 + v % 3;
            else cls = r & 15;
            switch ((int) cls) {
                case 0, 1, 2, 3, 4 -> bytes.push(v % 128);
                case 5 -> { bytes.push(0xc0 | ((v >>> 2) & 0x1f)); bytes.push(cont(v >>> 7)); }
                case 6 -> { bytes.push(0xe0 | ((v >>> 2) & 0x0f)); bytes.push(cont(v >>> 6)); bytes.push(cont(v >>> 12)); }
                case 7 -> bytes.push(cont(v >>> 2));
                case 8 -> bytes.push(0xc0 | ((v >>> 2) & 0x1f));
                case 9 -> bytes.push(0xe0 | ((v >>> 2) & 0x0f));
                case 10, 11 -> bytes.push(0xf0 | ((v >>> 2) & 0x0f));
                case 12 -> { bytes.push(0xc0 | ((v >>> 2) & 0x1f)); bytes.push(v >>> 7); }
                case 13 -> { bytes.push(0xe0 | ((v >>> 2) & 0x0f)); bytes.push(cont(v >>> 6)); bytes.push(v >>> 12); }
                default -> bytes.push(v);
            }
        }
        bytes.n = Math.min(bytes.n, len);
    }

    static void chunk(String mode, long c, Sink sink) throws IOException {
        Units units = new Units();
        Bytes bytes = new Bytes();
        char[] u2 = new char[2];
        switch (mode) {
            case "enc-singles" -> {
                sink.encode(u2, 0);
                for (int u = 0; u <= 0xffff; u++) { u2[0] = (char) u; sink.encode(u2, 1); }
            }
            case "enc-surrogates" -> {
                char h = (char) (0xd800 + c);
                for (int l = 0xdc00; l <= 0xdfff; l++) {
                    sink.encode(new char[] {h, (char) l}, 2);
                    sink.encode(new char[] {(char) l, h}, 2);
                }
                char l = (char) (0xdc00 + c);
                for (char s : new char[] {h, l}) {
                    sink.encode(new char[] {s}, 1);
                    sink.encode(new char[] {s, 'a'}, 2);
                    sink.encode(new char[] {'a', s}, 2);
                    sink.encode(new char[] {s, s}, 2);
                }
            }
            case "enc-pairs" -> {
                for (long a = c * 64; a < c * 64 + 64; a++) {
                    u2[0] = (char) a;
                    for (int b = 0; b <= 0xffff; b++) { u2[1] = (char) b; sink.encode(u2, 2); }
                }
            }
            case "enc-random" -> {
                SplitMix rng = new SplitMix(1, c);
                for (long i = 0; i < RANDOM_CHUNK; i++) { randomUnits(rng, units); sink.encode(units.a, units.n); }
            }
            case "enc-edge" -> {
                int[][] widths = {{'x', 1}, {0, 2}, {0x7ff, 2}, {0x800, 3}, {0xd800, 3}};
                for (int target = 65_533; target <= 65_538; target++) {
                    for (int[] w : widths) {
                        units.n = 0;
                        for (int i = 0; i < target / w[1]; i++) units.push(w[0]);
                        for (int i = 0; i < target % w[1]; i++) units.push('y');
                        sink.encode(units.a, units.n);
                    }
                }
            }
            case "dec-exhaustive" -> {
                byte[] b = new byte[3];
                if (c == 0) {
                    for (int a = 0; a <= 0xff; a++) { b[0] = (byte) a; sink.decode(b, 1); }
                    for (int a = 0; a <= 0xffff; a++) { b[0] = (byte) (a >>> 8); b[1] = (byte) a; sink.decode(b, 2); }
                } else {
                    for (int a = 0; a <= 0xffff; a++) {
                        b[0] = (byte) (c - 1); b[1] = (byte) (a >>> 8); b[2] = (byte) a;
                        sink.decode(b, 3);
                    }
                }
            }
            case "dec-random" -> {
                SplitMix rng = new SplitMix(2, c);
                for (long i = 0; i < RANDOM_CHUNK; i++) { randomBytes(rng, bytes); sink.decode(bytes.a, bytes.n); }
            }
            case "dec-valid" -> {
                SplitMix rng = new SplitMix(3, c);
                for (long i = 0; i < RANDOM_CHUNK; i++) {
                    randomUnits(rng, units);
                    sink.bytes.reset();
                    sink.data.writeUTF(new String(units.a, 0, units.n));
                    byte[] encoded = sink.bytes.toByteArray();
                    sink.fold(java.util.Arrays.copyOfRange(encoded, 2, encoded.length), encoded.length - 2);
                    sink.decode(java.util.Arrays.copyOfRange(encoded, 2, encoded.length), encoded.length - 2);
                }
            }
            default -> throw new IllegalArgumentException("unknown mode " + mode);
        }
    }

    /** The NBT writers' edge strings with the bytes `writeUTF` gives them, in the probe's `callers` format. */
    static void callers() throws IOException {
        int[][] widths = {{'x', 1}, {0, 2}, {0x800, 3}};
        StringBuilder out = new StringBuilder();
        for (int target = 65_533; target <= 65_538; target++) {
            for (int[] w : widths) {
                StringBuilder s = new StringBuilder();
                for (int i = 0; i < target / w[1]; i++) s.append((char) w[0]);
                for (int i = 0; i < target % w[1]; i++) s.append('y');
                ByteArrayOutputStream body = new ByteArrayOutputStream();
                String disk = "0800000000", network = "refused";
                try {
                    new DataOutputStream(body).writeUTF(s.toString());
                    String hex = hexBytes(body.toByteArray(), body.size());
                    disk = "080000" + hex;
                    network = "08" + hex;
                } catch (UTFDataFormatException e) {
                    // too long: the disk writer's empty string, the network writer's refusal
                }
                out.append(String.format("%d %04x disk=%s network=%s%n", target, w[0], disk, network));
            }
        }
        System.out.print(out);
    }

    public static void main(String[] args) throws Exception {
        if (args[0].equals("callers")) {
            callers();
            return;
        }
        if (args[0].equals("dump")) {
            Sink sink = new Sink(true);
            chunk(args[1], Long.parseLong(args[2]), sink);
            System.out.print(sink.dump);
            return;
        }
        String mode = args[0];
        long first = Long.parseLong(args[1]), count = Long.parseLong(args[2]);
        int threads = Math.max(1, Integer.parseInt(args[3]));
        AtomicLong next = new AtomicLong(first);
        List<long[]> results = new ArrayList<>();
        List<Throwable> errors = new ArrayList<>();
        List<Thread> workers = new ArrayList<>();
        for (int t = 0; t < threads; t++) {
            Thread worker = new Thread(() -> {
                try {
                    for (long c; (c = next.getAndIncrement()) < first + count; ) {
                        Sink sink = new Sink(false);
                        chunk(mode, c, sink);
                        synchronized (results) { results.add(new long[] {c, sink.records, sink.hash}); }
                    }
                } catch (Throwable e) {
                    synchronized (errors) { errors.add(e); }
                }
            });
            worker.start();
            workers.add(worker);
        }
        for (Thread worker : workers) worker.join();
        if (!errors.isEmpty()) throw new RuntimeException(errors.get(0));
        results.sort((a, b) -> Long.compare(a[0], b[0]));
        StringBuilder out = new StringBuilder();
        for (long[] r : results) out.append(r[0]).append(' ').append(r[1]).append(' ').append(String.format("%016x", r[2])).append('\n');
        System.out.print(out);
    }
}
