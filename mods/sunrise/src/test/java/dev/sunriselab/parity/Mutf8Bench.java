package dev.sunriselab.parity;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * The JVM side of the modified UTF-8 speed kit: DataOutputStream.writeUTF and DataInputStream.readUTF over the kit's
 * input sets, timed as the Rust loops are (per string, median of 400 batches after warm-up).
 * Usage: Mutf8Bench OP SET DIR ROUNDS [stepped]; "stepped" waits for a line on stdin before each round.
 */
public final class Mutf8Bench {
    static volatile long sink;

    /** A ByteArrayInputStream read again from its start. */
    static final class Input extends ByteArrayInputStream {
        Input(byte[] bytes) { super(bytes); }
        void rewind() { pos = 0; }
    }

    public static void main(String[] a) throws IOException {
        String op = a[0];
        ByteBuffer file = ByteBuffer.wrap(Files.readAllBytes(Path.of(a[2], a[1] + ".units"))).order(ByteOrder.LITTLE_ENDIAN);
        List<String> list = new ArrayList<>();
        while (file.hasRemaining()) {
            char[] units = new char[file.getInt()];
            for (int i = 0; i < units.length; i++) units[i] = file.getChar();
            list.add(new String(units));
        }
        String[] strings = list.toArray(new String[0]);
        ByteArrayOutputStream bos = new ByteArrayOutputStream(1 << 20);
        DataOutputStream out = new DataOutputStream(bos);
        for (String s : strings) out.writeUTF(s);
        Input input = new Input(bos.toByteArray());
        DataInputStream in = new DataInputStream(input);
        String[] slots = new String[strings.length];
        int rounds = Integer.parseInt(a[3]);
        boolean stepped = a.length > 4 && a[4].equals("stepped");
        java.io.BufferedReader stdin = new java.io.BufferedReader(new java.io.InputStreamReader(System.in));
        boolean encode = op.equals("encode");
        for (int round = 0; round < rounds; round++) {
            if (stepped && stdin.readLine() == null) return;
            for (int w = 0; w < 2000; w++) sink = encode ? encode(strings, bos, out) : decode(input, in, slots);
            double[] t = new double[400];
            for (int b = 0; b < 400; b++) {
                long s = System.nanoTime();
                sink = encode ? encode(strings, bos, out) : decode(input, in, slots);
                t[b] = (System.nanoTime() - s) / (double) strings.length;
            }
            Arrays.sort(t);
            System.out.printf("hotspot median %.4f ns  p99 %.4f ns%n", t[200], t[396]);
            System.out.flush();
        }
    }

    static long encode(String[] strings, ByteArrayOutputStream bos, DataOutputStream out) throws IOException {
        bos.reset();
        for (String s : strings) out.writeUTF(s);
        return bos.size();
    }

    static long decode(Input input, DataInputStream in, String[] slots) throws IOException {
        input.rewind();
        for (int i = 0; i < slots.length; i++) slots[i] = in.readUTF();
        return slots[slots.length - 1].length();
    }
}
