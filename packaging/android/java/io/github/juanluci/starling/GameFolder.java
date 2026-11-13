package io.github.juanluci.starling;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * The game folder as documents, which GameFolderProvider shows to Android's Files app. A document's
 * id is "starling" for the folder and "starling/<path>" for what is in it, so an id still works
 * after the folder moves to shared storage. Nothing here touches Android:
 * tests/test_android_game_folder.py runs it on a computer.
 *
 * <p>In shared storage the whole folder is the player's. In the app's own files it sits beside
 * their sign-ins, native builds and caches, so only PLAYER_FILES and LOGS show there, and those
 * names stay as they are.
 */
final class GameFolder {
    static final String ROOT = "starling";

    /** android.rs PLAYER_FILES, which the test compares with this. */
    private static final List<String> PLAYER_FILES = Arrays.asList(
            "saves",
            "backups",
            "resourcepacks",
            "datapacks",
            "shaderpacks",
            "horizon-mods",
            "screenshots",
            "servers.dat",
            "horizon-mods.txt",
            "horizon-graphics.txt",
            "horizon-extras.txt",
            "horizon-controller.txt",
            "horizon-player.txt",
            "horizon.txt",
            "touch-layout.json",
            "options.txt");

    /** Shown in the app's own files but never moved: what players need for a bug report. */
    private static final List<String> LOGS = Arrays.asList("logs", "crash-reports");

    /** Made when the app's own files are listed: nothing can be added at their top. */
    private static final String[] FOLDERS = {"saves", "resourcepacks", "shaderpacks", "horizon-mods"};

    /** How many taken names a new document tries before it gives up. */
    private static final int NAME_TRIES = 1000;

    private final File root;
    private final boolean ownFiles;

    GameFolder(File root, boolean ownFiles) {
        this.root = root;
        this.ownFiles = ownFiles;
    }

    /** The folder itself (DiagnosticsActivity's graphics API choice writes its settings file). */
    File root() {
        return root;
    }

    /**
     * The folder the game uses, as android.rs game_folder chooses it: `shared` (null when not
     * allowed), once the game has made it and moved the player's files there at a start, else
     * the app's own files. This never makes the shared folder: the game renames an older Horizon
     * folder to it only when it is missing.
     */
    static GameFolder choose(File own, String shared) {
        if (shared != null) {
            File folder = new File(shared);
            boolean moved = new File(folder, "options.txt").exists()
                    || !new File(own, "options.txt").exists();
            if (folder.isDirectory() && moved) {
                return new GameFolder(folder, false);
            }
        }
        return new GameFolder(own, true);
    }

    /** Whether a file or folder may have this name: one part of a path, inside its folder. */
    private static boolean validName(String name) {
        return !name.isEmpty() && !".".equals(name) && !"..".equals(name)
                && name.indexOf('/') < 0 && name.indexOf('\\') < 0 && name.indexOf('\0') < 0
                && name.getBytes(StandardCharsets.UTF_8).length <= 255;
    }

    /** The file of a document, which may not exist. Each part is checked: no way out, no links. */
    File file(String id) throws FileNotFoundException {
        if (!ROOT.equals(id) && !id.startsWith(ROOT + "/")) {
            throw new FileNotFoundException("Not in the game folder: " + id);
        }
        File file = root;
        refuseLink(file, id);
        if (ROOT.equals(id)) {
            return file;
        }
        String[] names = id.substring(ROOT.length() + 1).split("/", -1);
        if (ownFiles && !PLAYER_FILES.contains(names[0]) && !LOGS.contains(names[0])) {
            throw new FileNotFoundException("Not in the game folder: " + id);
        }
        for (String name : names) {
            if (!validName(name)) {
                throw new FileNotFoundException("Not in the game folder: " + id);
            }
            file = new File(file, name);
            refuseLink(file, id);
        }
        return file;
    }

    private static void refuseLink(File file, String id) throws FileNotFoundException {
        if (Files.isSymbolicLink(file.toPath())) {
            throw new FileNotFoundException("Not in the game folder: " + id);
        }
    }

    /**
     * The id of `target`, a path in this game folder, when the Files app may show it; else null.
     * Compared without links on the way (/sdcard, /data/data), so either spelling of a path works.
     */
    String idOf(File target) {
        String top;
        String path;
        try {
            top = root.getCanonicalPath();
            path = target.getCanonicalPath();
        } catch (IOException e) {
            return null;
        }
        if (!path.equals(top) && !path.startsWith(top + File.separator)) {
            return null;
        }
        String id = ROOT + path.substring(top.length()).replace(File.separatorChar, '/');
        try {
            file(id);
            return id;
        } catch (FileNotFoundException e) {
            return null;
        }
    }

    /** The folder a document is in. */
    static String parent(String id) {
        return id.substring(0, id.lastIndexOf('/'));
    }

    /** The documents in a folder, without links, special files and what is not the player's. */
    List<String> children(String id) throws IOException {
        File folder = file(id);
        if (ownFiles && ROOT.equals(id)) {
            for (String name : FOLDERS) {
                new File(root, name).mkdirs();
            }
        }
        String[] names = folder.list();
        if (names == null) {
            throw new FileNotFoundException(id);
        }
        List<String> children = new ArrayList<>();
        for (String name : names) {
            String child = id + "/" + name;
            try {
                File file = file(child);
                if (file.isDirectory() || file.isFile()) {
                    children.add(child);
                }
            } catch (FileNotFoundException e) {
                // Not shown.
            }
        }
        return children;
    }

    /** Whether a document keeps its name and place: the folder, and the app's own files' top. */
    boolean fixed(String id) {
        return ROOT.equals(id) || (ownFiles && id.indexOf('/') == id.lastIndexOf('/'));
    }

    /** Whether new documents may go in this folder. */
    boolean takesNew(String id) {
        return !(ownFiles && ROOT.equals(id));
    }

    /** A new file or folder, " (1)" added when the name is taken; its id. */
    String create(String parent, String name, boolean folder) throws IOException {
        File directory = file(parent);
        if (!takesNew(parent) || !validName(name) || !directory.isDirectory()) {
            throw new IOException("Cannot make " + name + " in " + parent);
        }
        int dot = name.lastIndexOf('.');
        String stem = dot > 0 ? name.substring(0, dot) : name;
        String extension = dot > 0 ? name.substring(dot) : "";
        for (int i = 0; i < NAME_TRIES; i++) {
            String free = i == 0 ? name : stem + " (" + i + ")" + extension;
            if (!validName(free)) {
                break;
            }
            File file = new File(directory, free);
            if (folder ? file.mkdir() : file.createNewFile()) {
                return parent + "/" + free;
            }
            if (!Files.exists(file.toPath(), LinkOption.NOFOLLOW_LINKS)) {
                throw new IOException("Cannot make " + name + " in " + parent);
            }
        }
        throw new IOException("No free name for " + name + " in " + parent);
    }

    /** A new name in the same folder, never over another document; the new id. */
    String rename(String id, String name) throws IOException {
        if (fixed(id) || !validName(name)) {
            throw new IOException("Cannot rename " + id + " to " + name);
        }
        String renamed = parent(id) + "/" + name;
        // Without REPLACE_EXISTING this fails on a taken name, and moves a link rather than its target.
        Files.move(file(id).toPath(), file(renamed).toPath());
        return renamed;
    }

    /** A file, or a folder with all it holds; a link inside goes, not what it points to. */
    void delete(String id) throws IOException {
        if (fixed(id)) {
            throw new IOException("Cannot delete " + id);
        }
        Files.walkFileTree(file(id).toPath(), new SimpleFileVisitor<Path>() {
            @Override
            public FileVisitResult visitFile(Path file, BasicFileAttributes attributes)
                    throws IOException {
                Files.delete(file);
                return FileVisitResult.CONTINUE;
            }

            @Override
            public FileVisitResult postVisitDirectory(Path folder, IOException failed)
                    throws IOException {
                if (failed != null) {
                    throw failed;
                }
                Files.delete(folder);
                return FileVisitResult.CONTINUE;
            }
        });
    }

    boolean isChild(String parent, String child) {
        try {
            file(parent);
            file(child);
            return child.startsWith(parent + "/");
        } catch (FileNotFoundException e) {
            return false;
        }
    }

    /** The ids from `top` (null: the game folder) down to `id`, both included. */
    List<String> path(String top, String id) throws FileNotFoundException {
        String from = top == null ? ROOT : top;
        file(id);
        if (!from.equals(id) && !isChild(from, id)) {
            throw new FileNotFoundException(id + " is not in " + from);
        }
        List<String> ids = new ArrayList<>();
        ids.add(from);
        int at = from.length();
        while (at < id.length()) {
            int next = id.indexOf('/', at + 1);
            at = next < 0 ? id.length() : next;
            ids.add(id.substring(0, at));
        }
        return ids;
    }
}
