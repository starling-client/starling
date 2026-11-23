package io.github.juanluci.starling;

import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.os.CancellationSignal;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.provider.DocumentsContract.Document;
import android.provider.DocumentsContract.Root;
import android.provider.DocumentsProvider;
import android.webkit.MimeTypeMap;
import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.util.Locale;

/**
 * The game folder as a place in Android's Files app and the system's file pickers: browse, open,
 * copy in and out, rename, delete. GameFolder holds the paths and the file operations; this is the
 * part that speaks to Android. MANAGE_DOCUMENTS in the manifest lets only the system bind it; an
 * app reaches a file or folder only after the player picks it there. Adapted from Horizon Android's
 * SunriseDocumentsProvider (THIRD_PARTY_NOTICES.md).
 */
public final class GameFolderProvider extends DocumentsProvider {
    private static final String[] ROOT_COLUMNS = {
        Root.COLUMN_ROOT_ID, Root.COLUMN_DOCUMENT_ID, Root.COLUMN_TITLE, Root.COLUMN_FLAGS,
        Root.COLUMN_ICON, Root.COLUMN_MIME_TYPES, Root.COLUMN_AVAILABLE_BYTES
    };
    private static final String[] DOCUMENT_COLUMNS = {
        Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE,
        Document.COLUMN_FLAGS, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED
    };

    @Override
    public boolean onCreate() {
        return true;
    }

    /** Looked up on each call: the player can allow shared storage while this process lives. */
    private GameFolder folder() {
        Context context = getContext();
        return GameFolder.choose(context.getFilesDir(), StarlingActivity.sharedFolder(context));
    }

    /** The manifest's authority, which follows the package: Starling and its Preview side by side. */
    private String authority() {
        return getContext().getPackageName() + ".documents";
    }

    /** The app's name, as the phone lists it: Starling, Starling Preview... */
    private String title() {
        Context context = getContext();
        return context.getApplicationInfo().loadLabel(context.getPackageManager()).toString();
    }

    private void changed(String folderId) {
        getContext().getContentResolver().notifyChange(
                DocumentsContract.buildChildDocumentsUri(authority(), folderId), null);
    }

    private static FileNotFoundException failed(IOException e) {
        if (e instanceof FileNotFoundException) {
            return (FileNotFoundException) e;
        }
        return new FileNotFoundException(e.toString());
    }

    @Override
    public Cursor queryRoots(String[] projection) throws FileNotFoundException {
        MatrixCursor cursor = new MatrixCursor(projection == null ? ROOT_COLUMNS : projection);
        MatrixCursor.RowBuilder row = cursor.newRow();
        row.add(Root.COLUMN_ROOT_ID, GameFolder.ROOT);
        row.add(Root.COLUMN_DOCUMENT_ID, GameFolder.ROOT);
        row.add(Root.COLUMN_TITLE, title());
        // SUPPORTS_CREATE: without it the system does not offer this place to copy to.
        row.add(Root.COLUMN_FLAGS,
                Root.FLAG_LOCAL_ONLY | Root.FLAG_SUPPORTS_CREATE | Root.FLAG_SUPPORTS_IS_CHILD);
        row.add(Root.COLUMN_ICON, getContext().getApplicationInfo().icon);
        row.add(Root.COLUMN_MIME_TYPES, "*/*");
        row.add(Root.COLUMN_AVAILABLE_BYTES, folder().file(GameFolder.ROOT).getUsableSpace());
        return cursor;
    }

    @Override
    public Cursor queryDocument(String id, String[] projection) throws FileNotFoundException {
        MatrixCursor cursor = new MatrixCursor(projection == null ? DOCUMENT_COLUMNS : projection);
        addRow(cursor, folder(), id);
        return cursor;
    }

    @Override
    public Cursor queryChildDocuments(String id, String[] projection, String sortOrder)
            throws FileNotFoundException {
        MatrixCursor cursor = new MatrixCursor(projection == null ? DOCUMENT_COLUMNS : projection);
        GameFolder folder = folder();
        try {
            for (String child : folder.children(id)) {
                try {
                    addRow(cursor, folder, child);
                } catch (FileNotFoundException e) {
                    // Gone since the listing.
                }
            }
        } catch (IOException e) {
            throw failed(e);
        }
        cursor.setNotificationUri(getContext().getContentResolver(),
                DocumentsContract.buildChildDocumentsUri(authority(), id));
        return cursor;
    }

    private void addRow(MatrixCursor cursor, GameFolder folder, String id)
            throws FileNotFoundException {
        File file = folder.file(id);
        boolean directory = file.isDirectory();
        if (!directory && !file.isFile()) {
            throw new FileNotFoundException(id);
        }
        int flags = directory
                ? (folder.takesNew(id) ? Document.FLAG_DIR_SUPPORTS_CREATE : 0)
                : Document.FLAG_SUPPORTS_WRITE;
        if (!folder.fixed(id)) {
            flags |= Document.FLAG_SUPPORTS_DELETE | Document.FLAG_SUPPORTS_RENAME;
        }
        MatrixCursor.RowBuilder row = cursor.newRow();
        row.add(Document.COLUMN_DOCUMENT_ID, id);
        row.add(Document.COLUMN_DISPLAY_NAME, GameFolder.ROOT.equals(id) ? title() : file.getName());
        row.add(Document.COLUMN_MIME_TYPE, directory ? Document.MIME_TYPE_DIR : mimeType(file.getName()));
        row.add(Document.COLUMN_FLAGS, flags);
        row.add(Document.COLUMN_SIZE, directory ? null : Long.valueOf(file.length()));
        row.add(Document.COLUMN_LAST_MODIFIED, file.lastModified());
    }

    /** By extension: the game's own kinds, then what Android knows. */
    private static String mimeType(String name) {
        int dot = name.lastIndexOf('.');
        String extension = dot < 0 ? "" : name.substring(dot + 1).toLowerCase(Locale.ROOT);
        switch (extension) {
            case "zip":
            case "smod":
            case "sshader":
                return "application/zip";
            case "json":
                return "application/json";
            case "txt":
            case "toml":
            case "mcmeta":
            case "log":
            case "wgsl":
            case "glsl":
            case "fsh":
            case "vsh":
                return "text/plain";
            default:
                String known = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension);
                return known != null ? known : "application/octet-stream";
        }
    }

    @Override
    public boolean isChildDocument(String parent, String child) {
        return folder().isChild(parent, child);
    }

    /** Lets the Files app show the way down to a document opened by its link. */
    @Override
    public DocumentsContract.Path findDocumentPath(String parent, String child)
            throws FileNotFoundException {
        return new DocumentsContract.Path(
                parent == null ? GameFolder.ROOT : null, folder().path(parent, child));
    }

    @Override
    public ParcelFileDescriptor openDocument(final String id, String mode, CancellationSignal signal)
            throws FileNotFoundException {
        if (signal != null) {
            signal.throwIfCanceled();
        }
        File file = folder().file(id);
        // Before opening: a write mode would make a file that is not there.
        if (!file.isFile()) {
            throw new FileNotFoundException(id);
        }
        int flags = ParcelFileDescriptor.parseMode(mode);
        if (mode.indexOf('w') < 0) {
            return ParcelFileDescriptor.open(file, flags);
        }
        // When the copying app closes it the listing updates, with the file's new size.
        try {
            return ParcelFileDescriptor.open(file, flags, new Handler(Looper.getMainLooper()),
                    new ParcelFileDescriptor.OnCloseListener() {
                        @Override
                        public void onClose(IOException e) {
                            changed(GameFolder.parent(id));
                        }
                    });
        } catch (IOException e) {
            throw failed(e);
        }
    }

    @Override
    public String createDocument(String parent, String mimeType, String name)
            throws FileNotFoundException {
        try {
            String id = folder().create(parent, name, Document.MIME_TYPE_DIR.equals(mimeType));
            changed(parent);
            return id;
        } catch (IOException e) {
            throw failed(e);
        }
    }

    @Override
    public String renameDocument(String id, String name) throws FileNotFoundException {
        try {
            String renamed = folder().rename(id, name);
            changed(GameFolder.parent(id));
            return renamed;
        } catch (IOException e) {
            throw failed(e);
        }
    }

    @Override
    public void deleteDocument(String id) throws FileNotFoundException {
        try {
            folder().delete(id);
            changed(GameFolder.parent(id));
        } catch (IOException e) {
            throw failed(e);
        }
    }
}
