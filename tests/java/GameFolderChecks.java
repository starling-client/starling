package io.github.juanluci.starling;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;

/**
 * GameFolder's rules on real files, run by tests/test_android_game_folder.py with an empty folder
 * to work in. Exits with a message at the first rule broken. Prints "symlinks skipped" where the
 * system does not let this process make a link.
 */
public final class GameFolderChecks {
    private interface Action {
        void run() throws IOException;
    }

    private static void check(boolean holds, String rule) {
        if (!holds) {
            System.err.println("broken: " + rule);
            System.exit(1);
        }
    }

    private static void refused(String rule, Action action) {
        try {
            action.run();
        } catch (IOException e) {
            return;
        }
        check(false, rule);
    }

    private static File write(File file, String text) throws IOException {
        file.getParentFile().mkdirs();
        Files.write(file.toPath(), text.getBytes(StandardCharsets.UTF_8));
        return file;
    }

    private static String read(File file) throws IOException {
        return new String(Files.readAllBytes(file.toPath()), StandardCharsets.UTF_8);
    }

    public static void main(String[] args) throws IOException {
        File work = new File(args[0]);
        choosing(new File(work, "choose"));
        ids(new File(work, "ids"));
        opening(new File(work, "opening"));
        ownFiles(new File(work, "own"));
        wardrobeImages(new File(work, "wardrobe"));
        sharedFolder(new File(work, "shared"));
        links(new File(work, "links"));
        System.out.println("ok");
    }

    private static void wardrobeImages(File root) throws IOException {
        File image = write(new File(root, "game-wardrobe/images/local.png"), "image");
        write(new File(root, "game-wardrobe/cloud/cached.png"), "cache");
        write(new File(root, "game-wardrobe/wardrobe.json"), "index");
        GameFolder folder = new GameFolder(root, true);
        check("starling/game-wardrobe/images/local.png".equals(folder.idOf(image)),
                "the wardrobe images have document IDs");
        check(folder.children("starling/game-wardrobe").equals(
                Arrays.asList("starling/game-wardrobe/images")), "only local images are exposed");
        check(folder.idOf(new File(root, "skins")) != null, "the separate skins folder can open");
    }

    /** Follows android.rs game_folder, and never makes the shared folder itself. */
    private static void choosing(File work) throws IOException {
        File own = new File(work, "files");
        File shared = new File(work, "Starling");
        own.mkdirs();
        check(!GameFolder.choose(own, null).takesNew(GameFolder.ROOT), "not allowed: own files");
        check(!GameFolder.choose(own, shared.getPath()).takesNew(GameFolder.ROOT),
                "allowed, no folder yet: own files");
        check(!shared.exists(), "the shared folder is the game's to make");
        shared.mkdirs();
        check(GameFolder.choose(own, shared.getPath()).takesNew(GameFolder.ROOT),
                "nothing to move: the shared folder");
        write(new File(own, "options.txt"), "");
        check(!GameFolder.choose(own, shared.getPath()).takesNew(GameFolder.ROOT),
                "files not moved yet: own files");
        write(new File(shared, "options.txt"), "");
        check(GameFolder.choose(own, shared.getPath()).takesNew(GameFolder.ROOT),
                "files moved: the shared folder");
    }

    /** Ids that leave the folder or are not canonical, for either kind of folder. */
    private static void ids(File work) throws IOException {
        File root = new File(work, "root");
        write(new File(root, "saves/World/level.dat"), "world");
        write(new File(work, "secret"), "secret");
        for (boolean ownFiles : new boolean[] {false, true}) {
            final GameFolder folder = new GameFolder(root, ownFiles);
            for (final String id : new String[] {
                "", "/", "x", "..", "starlingx", "starling/", "starling//saves", "starling/.",
                "starling/..", "starling/../secret", "starling/./saves", "starling/saves/",
                "starling/saves/..", "starling/saves/../../secret", "starling/saves/World/../..",
                "starling/saves\\..\\..\\secret", "starling/sa\0ves", "/starling/saves",
            }) {
                refused("the id \"" + id + "\" is out", new Action() {
                    @Override
                    public void run() throws IOException {
                        folder.file(id);
                    }
                });
            }
            check(folder.file("starling/saves/World/level.dat").isFile(), "a world's file is in");
            check(folder.isChild("starling", "starling/saves/World"), "a child");
            check(folder.isChild("starling/saves", "starling/saves/World"), "a child");
            check(!folder.isChild("starling/saves", "starling/savesX/World"), "another folder");
            check(!folder.isChild("starling/saves", "starling/saves"), "not its own child");
            check(!folder.isChild("starling/saves", "starling/saves/../resourcepacks"), "out");
            check(!folder.isChild("../x", "../x/y"), "out");
            check(folder.path(null, "starling/saves/World").equals(
                    Arrays.asList("starling", "starling/saves", "starling/saves/World")), "the path");
            check(folder.path("starling/saves", "starling/saves/World").equals(
                    Arrays.asList("starling/saves", "starling/saves/World")), "a path from a tree");
            check(folder.path(null, "starling").equals(Arrays.asList("starling")), "the root's");
            refused("no path from elsewhere", new Action() {
                @Override
                public void run() throws IOException {
                    folder.path("starling/resourcepacks", "starling/saves/World");
                }
            });
        }
    }

    /** The game's own paths as the documents the Files app opens on (StarlingActivity.openFolder). */
    private static void opening(File work) throws IOException {
        File root = new File(work, "root");
        write(new File(root, "resourcepacks/pack.zip"), "pack");
        write(new File(root, "accounts/account.json"), "token");
        write(new File(work, "rootx/other"), "other");
        for (boolean ownFiles : new boolean[] {false, true}) {
            GameFolder folder = new GameFolder(root, ownFiles);
            check(GameFolder.ROOT.equals(folder.idOf(root)), "the folder itself");
            check("starling/resourcepacks".equals(folder.idOf(new File(root, "resourcepacks"))),
                    "a folder in it");
            check("starling/resourcepacks".equals(
                    folder.idOf(new File(root, "saves/../resourcepacks"))), "a path with ..");
            check("starling/shaderpacks".equals(folder.idOf(new File(root, "shaderpacks"))),
                    "one not made yet");
            check(folder.idOf(work) == null, "the folder above");
            check(folder.idOf(new File(work, "rootx")) == null, "a name that starts the same");
            check(folder.idOf(new File(root, "../rootx/other")) == null, "out by ..");
        }
        check(new GameFolder(root, false).idOf(new File(root, "accounts")) != null,
                "shared storage is all the player's");
        check(new GameFolder(root, true).idOf(new File(root, "accounts")) == null,
                "the app's own sign-ins are not shown");
        check(new GameFolder(root, true).idOf(new File(root, "horizon-native-mods")) == null,
                "nor its native mods' folder: the game offers the Starling folder for it");
    }

    /** The app's own files: their sign-ins and builds out of reach, the top's names fixed. */
    private static void ownFiles(File root) throws IOException {
        final GameFolder folder = new GameFolder(root, true);
        write(new File(root, "accounts/account.json"), "token");
        write(new File(root, "native/game.so"), "code");
        write(new File(root, "horizon-control.json"), "secret");
        write(new File(root, "options.txt"), "options");
        write(new File(root, "saves/World/level.dat"), "world");
        for (final String id : new String[] {
            "starling/accounts", "starling/accounts/account.json", "starling/native/game.so",
            "starling/horizon-control.json", "starling/moved-to-shared", "starling/.horizon-cache",
        }) {
            refused(id + " is private", new Action() {
                @Override
                public void run() throws IOException {
                    folder.file(id);
                }
            });
        }
        List<String> top = folder.children("starling");
        check(top.contains("starling/options.txt") && top.contains("starling/saves"), "listed");
        for (String made : new String[] {"resourcepacks", "shaderpacks", "horizon-mods"}) {
            check(top.contains("starling/" + made), made + " is made to copy into");
        }
        check(!top.contains("starling/accounts") && !top.contains("starling/native")
                && !top.contains("starling/horizon-control.json"), "the private ones are not listed");
        check(!new File(root, "logs").exists() && !new File(root, "crash-reports").exists(),
                "the game makes its own logs folders");
        write(new File(root, "logs/latest.log"), "log");
        write(new File(root, "crash-reports/crash-1.txt"), "crash");
        top = folder.children("starling");
        check(top.contains("starling/logs") && top.contains("starling/crash-reports"), "logs listed");
        check(!top.contains("starling/accounts") && !top.contains("starling/native"),
                "still no accounts or native builds");
        check(folder.children("starling/logs").contains("starling/logs/latest.log"), "a log listed");
        check(read(folder.file("starling/logs/latest.log")).equals("log"), "a log read");
        check(read(folder.file("starling/crash-reports/crash-1.txt")).equals("crash"), "a crash read");
        for (final String id : new String[] {"starling/logs", "starling/crash-reports"}) {
            check(folder.fixed(id), id + " is fixed");
            refused(id + " keeps its name", new Action() {
                @Override
                public void run() throws IOException {
                    folder.rename(id, "accounts");
                }
            });
            refused(id + " stays", new Action() {
                @Override
                public void run() throws IOException {
                    folder.delete(id);
                }
            });
        }
        check(new File(root, "logs/latest.log").isFile()
                && new File(root, "crash-reports/crash-1.txt").isFile(), "not deleted");
        refused("no way from the logs to the accounts", new Action() {
            @Override
            public void run() throws IOException {
                folder.file("starling/logs/../accounts/account.json");
            }
        });
        check(!folder.takesNew("starling") && folder.takesNew("starling/saves"), "the top is closed");
        refused("nothing new at the top", new Action() {
            @Override
            public void run() throws IOException {
                folder.create("starling", "accounts", true);
            }
        });
        check(folder.fixed("starling/saves") && folder.fixed("starling/options.txt"), "fixed");
        check(!folder.fixed("starling/saves/World"), "a world is the player's");
        refused("the top keeps its names", new Action() {
            @Override
            public void run() throws IOException {
                folder.rename("starling/saves", "accounts");
            }
        });
        refused("the top stays", new Action() {
            @Override
            public void run() throws IOException {
                folder.delete("starling/saves");
            }
        });
        refused("no way out by a new name", new Action() {
            @Override
            public void run() throws IOException {
                folder.rename("starling/saves/World", "../../accounts");
            }
        });
        refused("no way out by a new document", new Action() {
            @Override
            public void run() throws IOException {
                folder.create("starling/saves", "../../accounts/new", false);
            }
        });
        String copy = folder.create("starling/saves", "Copied", true);
        check(new File(root, "saves/Copied").isDirectory() && copy.equals("starling/saves/Copied"),
                "a world copied in");
        check(read(new File(root, "accounts/account.json")).equals("token"), "accounts untouched");
    }

    /** The shared folder: all of it the player's, nothing lost to a name. */
    private static void sharedFolder(File root) throws IOException {
        final GameFolder folder = new GameFolder(root, false);
        root.mkdirs();
        check(folder.takesNew("starling") && folder.fixed("starling"), "the root stays, takes new");
        check(!folder.fixed("starling/saves"), "the rest is the player's");
        check(folder.create("starling", "pack.zip", false).equals("starling/pack.zip"), "new file");
        write(new File(root, "pack.zip"), "first");
        check(folder.create("starling", "pack.zip", false).equals("starling/pack (1).zip"),
                "a taken name gets another");
        check(folder.create("starling", "pack.zip", false).equals("starling/pack (2).zip"),
                "and another");
        check(read(new File(root, "pack.zip")).equals("first"), "not written over");
        check(folder.create("starling", "saves", true).equals("starling/saves"), "new folder");
        check(folder.create("starling", "saves", true).equals("starling/saves (1)"), "taken too");
        check(folder.create("starling", ".nomedia", false).equals("starling/.nomedia"), "a dot file");
        check(folder.create("starling", ".nomedia", false).equals("starling/.nomedia (1)"),
                "a dot is no extension");
        for (final String name : new String[] {"", ".", "..", "a/b", "../x", "a\\b", "a\0b"}) {
            refused("the name \"" + name + "\" is out", new Action() {
                @Override
                public void run() throws IOException {
                    folder.create("starling", name, false);
                }
            });
            refused("the name \"" + name + "\" is out", new Action() {
                @Override
                public void run() throws IOException {
                    folder.rename("starling/pack.zip", name);
                }
            });
        }
        refused("not into a file", new Action() {
            @Override
            public void run() throws IOException {
                folder.create("starling/pack.zip", "x", false);
            }
        });
        refused("a rename does not write over", new Action() {
            @Override
            public void run() throws IOException {
                folder.rename("starling/pack (1).zip", "pack.zip");
            }
        });
        check(read(new File(root, "pack.zip")).equals("first"), "still the first");
        check(folder.rename("starling/pack (1).zip", "other.zip").equals("starling/other.zip"),
                "renamed");
        check(new File(root, "other.zip").isFile() && !new File(root, "pack (1).zip").exists(),
                "renamed on disk");
        write(new File(root, "saves/World/region/r.0.0.mca"), "chunks");
        check(folder.rename("starling/saves/World", "Renamed").equals("starling/saves/Renamed"),
                "a folder renamed");
        check(new File(root, "saves/Renamed/region/r.0.0.mca").isFile(), "with what it holds");
        folder.delete("starling/other.zip");
        check(!new File(root, "other.zip").exists(), "a file deleted");
        folder.delete("starling/saves/Renamed");
        check(!new File(root, "saves/Renamed").exists(), "a full folder deleted");
        check(new File(root, "saves").isDirectory(), "not its parent");
        refused("the root stays", new Action() {
            @Override
            public void run() throws IOException {
                folder.delete("starling");
            }
        });
        refused("the root keeps its name", new Action() {
            @Override
            public void run() throws IOException {
                folder.rename("starling", "other");
            }
        });
        refused("nothing to delete", new Action() {
            @Override
            public void run() throws IOException {
                folder.delete("starling/none");
            }
        });
    }

    /** A link in the folder is not followed by anything here. */
    private static void links(File work) throws IOException {
        File root = new File(work, "root");
        File outside = new File(work, "outside");
        write(new File(outside, "secret"), "secret");
        write(new File(root, "saves/World/level.dat"), "world");
        Path toFolder = new File(root, "saves/link").toPath();
        try {
            Files.createSymbolicLink(toFolder, outside.toPath().toAbsolutePath());
        } catch (IOException | UnsupportedOperationException e) {
            System.out.println("symlinks skipped");
            return;
        }
        Files.createSymbolicLink(new File(root, "saves/World/file-link").toPath(),
                new File(outside, "secret").toPath().toAbsolutePath());
        Files.createSymbolicLink(new File(root, "saves/World/folder-link").toPath(),
                outside.toPath().toAbsolutePath());
        final GameFolder folder = new GameFolder(root, false);
        for (final String id : new String[] {
            "starling/saves/link", "starling/saves/link/secret", "starling/saves/World/file-link",
        }) {
            refused(id + " is a link", new Action() {
                @Override
                public void run() throws IOException {
                    folder.file(id);
                }
            });
        }
        check(!folder.children("starling/saves").contains("starling/saves/link"), "not listed");
        List<String> world = folder.children("starling/saves/World");
        check(world.size() == 1 && world.contains("starling/saves/World/level.dat"), "not listed");
        check(!folder.isChild("starling", "starling/saves/link/secret"), "not a child");
        refused("no new document through a link", new Action() {
            @Override
            public void run() throws IOException {
                folder.create("starling/saves/link", "new", false);
            }
        });
        check(!new File(outside, "new").exists(), "nothing made outside");
        refused("no rename onto a link", new Action() {
            @Override
            public void run() throws IOException {
                folder.rename("starling/saves/World", "link");
            }
        });
        refused("a link is no document to delete", new Action() {
            @Override
            public void run() throws IOException {
                folder.delete("starling/saves/link");
            }
        });
        folder.delete("starling/saves/World");
        check(!new File(root, "saves/World").exists(), "the folder with links deleted");
        check(read(new File(outside, "secret")).equals("secret"), "what they point to stays");
        // A folder swapped for a link: its top too.
        File swapped = new File(work, "swapped");
        Files.createSymbolicLink(swapped.toPath(), outside.toPath().toAbsolutePath());
        final GameFolder linked = new GameFolder(swapped, false);
        refused("a game folder that is a link", new Action() {
            @Override
            public void run() throws IOException {
                linked.file("starling");
            }
        });
    }
}
