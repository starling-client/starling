import java.io.BufferedOutputStream;
import java.io.DataOutputStream;
import java.io.FileOutputStream;
import java.io.IOException;

/**
 * The vanilla side of tools/jmath_fuzz.py's game functions: vanilla 26.3's own methods on a seeded stream of inputs,
 * their results as raw bits (8 bytes each, little-endian). Needs the server jar and its libraries on the classpath.
 *
 *     java -cp classes:server.jar:libraries VanillaMathFuzz FUNCTION SEED COUNT OUT
 *
 * The inputs come from SplitMix64 and the classes crates/sunrise-world/examples/probe/vanilla_math_fuzz.rs
 * regenerates: i % 4 == 0 a float in the usual range of the argument, 1 a random exponent in [-30, 30], 2 any float
 * bit pattern, 3 an edge angle nudged by up to 3 ulps.
 */
public final class VanillaMathFuzz {
    private long state;

    private VanillaMathFuzz(long seed) {
        state = seed;
    }

    private long next() {
        long z = (state += 0x9E3779B97F4A7C15L);
        z = (z ^ (z >>> 30)) * 0xBF58476D1CE4E5B9L;
        z = (z ^ (z >>> 27)) * 0x94D049BB133111EBL;
        return z ^ (z >>> 31);
    }

    static final float[] EDGES = {
        0f, -0f, 1f, -1f, 45f, -45f, 89.9f, 90f, -90f, 180f, -180f, 270f, 360f, -360f, 720f, 22.5f, 0.5f,
        Float.MIN_VALUE, Float.MAX_VALUE, 1e9f, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY, Float.NaN,
    };

    /** A float input in [lo, hi] for class 0. */
    float angle(long i, float lo, float hi) {
        switch ((int) (i & 3)) {
            case 0: {
                double u = (next() >>> 11) * 0x1.0p-53;
                return (float) (lo + u * (hi - lo));
            }
            case 1: {
                long bits = next();
                int exponent = 127 - 30 + (int) Long.remainderUnsigned(bits >>> 40, 61);
                return Float.intBitsToFloat((int) (bits & 0x807FFFFFL) | (exponent << 23));
            }
            case 2:
                return Float.intBitsToFloat((int) next());
            default: {
                float edge = EDGES[(int) Long.remainderUnsigned(next(), EDGES.length)];
                int nudge = (int) Long.remainderUnsigned(next(), 7) - 3;
                if (Float.isNaN(edge) || Float.isInfinite(edge)) return edge;
                return Float.intBitsToFloat(Float.floatToRawIntBits(edge) + nudge);
            }
        }
    }

    /** A double input in [lo, hi] for class 0; the other classes as for {@link #angle}, widened. */
    double coord(long i, double lo, double hi) {
        if ((i & 3) == 0) {
            double u = (next() >>> 11) * 0x1.0p-53;
            return lo + u * (hi - lo);
        }
        return angle(i, 0f, 0f);
    }

    static int outputs(String f) {
        switch (f) {
            case "view_vector": return 3;
            case "vec3_yrot": return 3;
            case "projectile_rot": return 2;
            case "input_vector": return 3;
            case "blocking_angle": return 1;
            case "mth_rounding": return 4;
            default: throw new IllegalArgumentException("unknown function " + f);
        }
    }

    static java.lang.reflect.Method entityStatic(String name, Class<?>... types) {
        try {
            var m = net.minecraft.world.entity.Entity.class.getDeclaredMethod(name, types);
            m.setAccessible(true);
            return m;
        } catch (NoSuchMethodException e) {
            throw new IllegalStateException(e);
        }
    }

    static final java.lang.reflect.Method INPUT_VECTOR = entityStatic("getInputVector",
        net.minecraft.world.phys.Vec3.class, float.class, float.class);

    double[] apply(String f, long i) throws ReflectiveOperationException {
        switch (f) {
            case "view_vector": {
                float xRot = angle(i, -90f, 90f);
                float yRot = angle(i, -360f, 360f);
                var v = net.minecraft.world.entity.Entity.calculateViewVector(xRot, yRot);
                return new double[] {v.x, v.y, v.z};
            }
            case "vec3_yrot": {
                // Entity.getPassengerRidingPosition's attachment turn: yRot(-yRot * (float)(PI / 180)).
                double x = coord(i, -4, 4), y = coord(i, -4, 4), z = coord(i, -4, 4);
                float yRot = angle(i, -360f, 360f);
                var v = new net.minecraft.world.phys.Vec3(x, y, z).yRot(-yRot * (float) (Math.PI / 180.0));
                return new double[] {v.x, v.y, v.z};
            }
            case "projectile_rot": {
                // Projectile.updateRotation's targets: (float)(Mth.atan2(..) * 57.2957763671875), the folded
                // 180.0F / (float)Math.PI its bytecode multiplies by; yaw from (x, z), pitch from (y, horizontal).
                double x = coord(i, -3, 3), y = coord(i, -3, 3), z = coord(i, -3, 3);
                var m = new net.minecraft.world.phys.Vec3(x, y, z);
                float yRot = (float) (net.minecraft.util.Mth.atan2(m.x, m.z) * 57.2957763671875);
                float xRot = (float) (net.minecraft.util.Mth.atan2(m.y, m.horizontalDistance()) * 57.2957763671875);
                return new double[] {yRot, xRot};
            }
            case "input_vector": {
                // Entity.getInputVector(input, speed, yRot), private static.
                double x = coord(i, -2, 2), y = coord(i, -2, 2), z = coord(i, -2, 2);
                float speed = angle(i, 0f, 1f), yRot = angle(i, -360f, 360f);
                var v = (net.minecraft.world.phys.Vec3) INPUT_VECTOR.invoke(null, new net.minecraft.world.phys.Vec3(x, y, z), speed, yRot);
                return new double[] {v.x, v.y, v.z};
            }
            case "blocking_angle": {
                // LivingEntity.applyItemBlocking's angle, in its bytecode's order: calculateViewVector(0, yHeadRot),
                // the source minus the position flattened and normalized, its dot with the view, Math.acos.
                var source = new net.minecraft.world.phys.Vec3(coord(i, -8, 8), coord(i, -8, 8), coord(i, -8, 8));
                var position = new net.minecraft.world.phys.Vec3(coord(i, -8, 8), coord(i, -8, 8), coord(i, -8, 8));
                float yHeadRot = angle(i, -360f, 360f);
                var view = net.minecraft.world.entity.Entity.calculateViewVector(0.0F, yHeadRot);
                var d = source.subtract(position);
                var flat = new net.minecraft.world.phys.Vec3(d.x, 0.0, d.z).normalize();
                return new double[] {Math.acos(flat.dot(view))};
            }
            case "mth_rounding": {
                // Mth.floor and Mth.ceil of a float and of a double, from -3e9 to 3e9 in class 0 (past the int
                // range on both sides).
                float x = angle(i, -3e9f, 3e9f);
                double d = coord(i, -3e9, 3e9);
                return new double[] {
                    net.minecraft.util.Mth.floor(x), net.minecraft.util.Mth.ceil(x),
                    net.minecraft.util.Mth.floor(d), net.minecraft.util.Mth.ceil(d),
                };
            }
            default: throw new IllegalArgumentException("unknown function " + f);
        }
    }

    public static void main(String[] args) throws IOException, ReflectiveOperationException {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        String f = args[0];
        VanillaMathFuzz random = new VanillaMathFuzz(Long.parseUnsignedLong(args[1]));
        long count = Long.parseLong(args[2]);
        byte[] buffer = new byte[8 * outputs(f)];
        try (DataOutputStream out = new DataOutputStream(new BufferedOutputStream(new FileOutputStream(args[3]), 1 << 20))) {
            for (long i = 0; i < count; i++) {
                double[] result = random.apply(f, i);
                for (int k = 0; k < result.length; k++) {
                    long bits = Double.doubleToRawLongBits(result[k]);
                    for (int b = 0; b < 8; b++) buffer[8 * k + b] = (byte) (bits >>> (8 * b));
                }
                out.write(buffer);
            }
        }
    }
}
