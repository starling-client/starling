package io.github.juanluci.starling;

import android.content.Context;
import android.content.pm.PackageManager;
import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.regex.Pattern;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * The game with native mods (game/native_builds.rs): a build of this version's game library with
 * the player's native mods compiled in, downloaded to files/native/KEY/ and chosen by
 * files/native/current. StarlingActivity loads it in place of the APK's own library. A build is
 * only loaded by the install of the APK it was made under (its `apk` file, `stamp`): after any
 * install, even of the same version, it waits for its rebuild. One that stopped twice before its
 * first frame is set aside, with a notice the game shows.
 */
final class NativeMods {
    private NativeMods() {}

    static final String LIBRARY = "libsunrise_android.so";
    /** Beside a build's library: the `stamp` of the install it was made under (native_builds.rs). */
    private static final String APK = "apk";
    /** Starts that ended before the first frame, after which the build is set aside. */
    private static final int TRIES = 2;
    private static final Pattern BUILD_SERVER_ADDRESS = Pattern.compile(
            "https://[a-z0-9]([a-z0-9.-]{0,251}[a-z0-9])?(:[0-9]{1,5})?/?", Pattern.CASE_INSENSITIVE);

    private static File dir(Context context) {
        return new File(context.getFilesDir(), "native");
    }

    /**
     * The chosen build's library for this `version` (the APK's versionName), or null to play the
     * APK's own; counts the start, which `started` ends.
     */
    static File chosen(Context context, String version) {
        File dir = dir(context);
        String key = read(new File(dir, "current"));
        if (key == null || key.isEmpty() || key.contains("/") || key.startsWith(".")) {
            return null;
        }
        File build = new File(dir, key);
        File library = new File(build, LIBRARY);
        if (!library.isFile() || version == null || !version.equals(read(new File(build, "for")))) {
            return null;
        }
        // Every test build of a version has the same versionName and versionCode: a build made
        // under another install has another commit's code, and the game says it is out of date.
        String stamp = stamp(context);
        if (stamp == null || !stamp.equals(read(new File(build, APK)))) {
            return null;
        }
        File starts = new File(dir, "starts");
        int count = 0;
        try {
            count = Integer.parseInt(String.valueOf(read(starts)));
        } catch (NumberFormatException e) {
            // None counted.
        }
        if (count >= TRIES) {
            setAside(dir, "The build with native mods closed twice before it showed, so "
                    + "Starling started without it.");
            return null;
        }
        write(starts, Integer.toString(count + 1));
        return library;
    }

    /**
     * This install of the APK: when Android installed or last updated it, which changes with every
     * APK put on the phone. The game writes it beside each build it downloads (STARLING_APK).
     */
    static String stamp(Context context) {
        try {
            return Long.toString(context.getPackageManager()
                    .getPackageInfo(context.getPackageName(), 0).lastUpdateTime);
        } catch (PackageManager.NameNotFoundException e) {
            return null;
        }
    }

    /** The game showed its first frame: the build started. */
    static void started(Context context) {
        new File(dir(context), "starts").delete();
    }

    /**
     * What the build at `library` says its mods need of Android, each with the mods that need it,
     * in build.json's order ({"microphone": ["voicechat"]}): native_build.sh finds it in their
     * source (native_mod_permissions.py); none from a build made before it did, or unreadable.
     */
    static Map<String, List<String>> permissions(File library) {
        Map<String, List<String>> wanted = new LinkedHashMap<>();
        String text = read(new File(library.getParentFile(), "build.json"));
        if (text == null) {
            return wanted;
        }
        try {
            JSONArray mods = new JSONObject(text).optJSONArray("mods");
            for (int i = 0; mods != null && i < mods.length(); i++) {
                JSONObject mod = mods.optJSONObject(i);
                JSONArray needs = mod == null ? null : mod.optJSONArray("permissions");
                for (int j = 0; needs != null && j < needs.length(); j++) {
                    String permission = needs.optString(j);
                    if (!wanted.containsKey(permission)) {
                        wanted.put(permission, new ArrayList<String>());
                    }
                    wanted.get(permission).add(mod.optString("id"));
                }
            }
        } catch (JSONException e) {
            // A damaged build.json asks for nothing.
        }
        return wanted;
    }

    /** Whether the player said Not now to the build's mods' permissions: not asked again for it. */
    static boolean permissionsDeclined(File library) {
        return new File(library.getParentFile(), "permissions-declined").exists();
    }

    static void declinePermissions(File library) {
        write(new File(library.getParentFile(), "permissions-declined"), "");
    }

    /** The build could not be loaded: set aside, saying why. */
    static void failed(Context context, String why) {
        setAside(dir(context), "The build with native mods could not be loaded (" + why
                + "), so Starling started without it.");
    }

    /**
     * Whether `text` is a build server's address, as game/native_builds.rs takes one: HTTPS to a
     * host, maybe with a port, and nothing else.
     */
    static boolean isBuildServer(String text) {
        return BUILD_SERVER_ADDRESS.matcher(text.trim()).matches();
    }

    /** Native mods compile on the build server at `address`, or on Starling's with null. */
    static void chooseBuildServer(Context context, String address) {
        File file = new File(dir(context), "build-server");
        if (address == null) {
            file.delete();
        } else {
            write(file, address.trim());
        }
    }

    private static void setAside(File dir, String notice) {
        new File(dir, "current").delete();
        new File(dir, "starts").delete();
        write(new File(dir, "notice"), notice);
    }

    /** A small text file's trimmed contents, null when it cannot be read. */
    private static String read(File file) {
        try {
            return new String(Files.readAllBytes(file.toPath()), StandardCharsets.UTF_8).trim();
        } catch (IOException e) {
            return null;
        }
    }

    private static void write(File file, String text) {
        try {
            file.getParentFile().mkdirs();
            Files.write(file.toPath(), text.getBytes(StandardCharsets.UTF_8));
        } catch (IOException e) {
            // Best effort: the count or notice is lost.
        }
    }
}
