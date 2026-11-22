import java.io.BufferedOutputStream;
import java.io.DataOutputStream;
import java.io.FileOutputStream;
import java.io.IOException;

/**
 * The JVM side of tools/jmath_fuzz.py: java.lang.Math's results for a seeded stream of inputs, as raw bits.
 *
 *     java JmathFuzz FUNCTION SEED COUNT OUT
 *
 * Writes COUNT results (8 bytes each, little-endian Double.doubleToRawLongBits) to OUT. The inputs come from
 * SplitMix64 and the same four classes as crates/sunrise-core/examples/jmath_fuzz.rs, which regenerates them:
 * i % 4 == 0 a uniform value in the function's usual range, 1 a random exponent in [-60, 60], 2 any bit pattern
 * (subnormals, infinities and NaNs included), 3 an edge value nudged by up to 3 ulps. "mthtable" is Mth's sine table,
 * entry i % 65536 as (float) Math.sin(i / 10430.378350470453), with no draws.
 */
public final class JmathFuzz {
    private long state;

    private JmathFuzz(long seed) {
        state = seed;
    }

    private long next() {
        long z = (state += 0x9E3779B97F4A7C15L);
        z = (z ^ (z >>> 30)) * 0xBF58476D1CE4E5B9L;
        z = (z ^ (z >>> 27)) * 0x94D049BB133111EBL;
        return z ^ (z >>> 31);
    }

    static final double[] EDGES = {
        0.0, -0.0, Double.MIN_VALUE, -Double.MIN_VALUE, Double.MIN_NORMAL, -Double.MIN_NORMAL,
        Math.nextDown(Double.MIN_NORMAL), 1.0, -1.0, 0.5, -0.5, 2.0, -2.0, 10.0, 0.1,
        Math.PI / 4, Math.PI / 2, Math.PI, 3 * Math.PI / 2, 2 * Math.PI, -Math.PI / 2, -Math.PI,
        (double) (float) (Math.PI * 2), 0x1.0p-26, 0x1.0p-27, 0x1.0p20, 0x1.0p52, 0x1.0p63, 1e9, 1e300, -1e300,
        709.782712893384, 709.79, -745.1332191019411, -745.14, 0.7853981633974483, 2.356194490192345,
        Double.MAX_VALUE, -Double.MAX_VALUE, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY, Double.NaN,
    };

    /** The function's usual range, as tools/jmath_fuzz.py and the Rust side define it. */
    static double[] range(String f) {
        switch (f) {
            case "exp": return new double[] {-745.0, 710.0};
            case "log": case "log10": case "cbrt": return new double[] {0.0, 1e6};
            case "asin": case "acos": return new double[] {-1.0, 1.0};
            case "pow": return new double[] {0.0, 16.0};
            default: return new double[] {-8 * Math.PI, 8 * Math.PI};
        }
    }

    double input(String f, long i, boolean second) {
        // Mth's sine table: entry i, as Mth's static initializer computes it (no draws).
        if (f.equals("mthtable")) return (i & 65535) / 10430.378350470453;
        double[] r = range(f);
        switch ((int) (i & 3)) {
            case 0: {
                double u = (next() >>> 11) * 0x1.0p-53;
                return second && f.equals("pow") ? -16.0 + u * 32.0 : r[0] + u * (r[1] - r[0]);
            }
            case 1: {
                long bits = next();
                long exponent = 1023 - 60 + Long.remainderUnsigned(bits >>> 52, 121);
                return Double.longBitsToDouble((bits & 0x800FFFFFFFFFFFFFL) | (exponent << 52));
            }
            case 2:
                return Double.longBitsToDouble(next());
            default: {
                double edge = EDGES[(int) Long.remainderUnsigned(next(), EDGES.length)];
                long nudge = Long.remainderUnsigned(next(), 7) - 3;
                if (Double.isNaN(edge) || Double.isInfinite(edge)) return edge;
                return Double.longBitsToDouble(Double.doubleToRawLongBits(edge) + nudge);
            }
        }
    }

    static double apply(String f, double x, double y) {
        switch (f) {
            case "sin": return Math.sin(x);
            case "cos": return Math.cos(x);
            case "tan": return Math.tan(x);
            case "exp": return Math.exp(x);
            case "log": return Math.log(x);
            case "log10": return Math.log10(x);
            case "pow": return Math.pow(x, y);
            case "cbrt": return Math.cbrt(x);
            case "atan2": return Math.atan2(x, y);
            case "atan": return Math.atan(x);
            case "asin": return Math.asin(x);
            case "acos": return Math.acos(x);
            case "hypot": return Math.hypot(x, y);
            case "mthtable": return (float) Math.sin(x);
            default: throw new IllegalArgumentException("unknown function " + f);
        }
    }

    static boolean binary(String f) {
        return f.equals("pow") || f.equals("atan2") || f.equals("hypot");
    }

    public static void main(String[] args) throws IOException {
        String f = args[0];
        JmathFuzz random = new JmathFuzz(Long.parseUnsignedLong(args[1]));
        long count = Long.parseLong(args[2]);
        byte[] buffer = new byte[8];
        try (DataOutputStream out = new DataOutputStream(new BufferedOutputStream(new FileOutputStream(args[3]), 1 << 20))) {
            for (long i = 0; i < count; i++) {
                double x = random.input(f, i, false);
                double y = binary(f) ? random.input(f, i, true) : 0.0;
                long bits = Double.doubleToRawLongBits(apply(f, x, y));
                for (int b = 0; b < 8; b++) buffer[b] = (byte) (bits >>> (8 * b));
                out.write(buffer);
            }
        }
    }
}
