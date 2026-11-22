package io.github.juanluci.starling;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.database.Cursor;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.RectF;
import android.graphics.Typeface;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.SystemClock;
import android.provider.OpenableColumns;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * What runs before the game (StarlingActivity) when it is handed something, in a process and a
 * task of its own: a file opened with Starling or shared to it (a resource pack, a world, a mod,
 * the prepared resources) is copied into the game's import inbox, which the game installs from
 * (crates/horizon-client/src/import.rs); and a restart the game asked for starts it again once
 * its old process has ended.
 */
public final class LauncherActivity extends Activity {
    /** The game's process id: start the game again once it has ended. */
    static final String AFTER = "io.github.juanluci.starling.AFTER";
    /** What the restart screen says while the game starts again (StarlingActivity.restart). */
    static final String MESSAGE = "io.github.juanluci.starling.MESSAGE";

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        Intent intent = getIntent();
        final int after = intent == null ? 0 : intent.getIntExtra(AFTER, 0);
        Uri file = handedFile(intent);
        if (after != 0) {
            showRestarting(intent.getStringExtra(MESSAGE));
            new Thread(new Runnable() {
                @Override
                public void run() {
                    // A few seconds at most: the game saves and ends as its activity finishes.
                    for (int i = 0; i < 100 && new File("/proc/" + after).exists(); i++) {
                        try {
                            Thread.sleep(50);
                        } catch (InterruptedException e) {
                            break;
                        }
                    }
                    runOnUiThread(new Runnable() {
                        @Override
                        public void run() {
                            play();
                        }
                    });
                }
            }, "starling-restart").start();
        } else if (file != null) {
            final AlertDialog copying =
                    new AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
                            .setMessage("Handing the file to Starling...")
                            .setCancelable(false)
                            .show();
            receive(this, file, new Runnable() {
                @Override
                public void run() {
                    copying.dismiss();
                    play();
                }
            });
        } else if (StartupRecovery.interrupted(this)) {
            startActivity(new Intent(this, DiagnosticsActivity.class)
                    .putExtra(StartupRecovery.RECOVERY, true));
            finish();
        } else {
            play();
        }
    }

    private void play() {
        // The game's own task (singleTask): it stays in the recents, this one does not.
        Intent game = new Intent(this, GameActivity.class);
        game.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        startActivity(game);
        finish();
        // Straight into the game's black first frame, no window animation between.
        overridePendingTransition(0, 0);
    }

    /**
     * While the game starts again (a world whose mods are not the running ones): a screen of its
     * own, black, landscape and without bars as the game is, so the phone does not show what is
     * under this see-through activity between the two processes, as if the game had closed.
     */
    @SuppressWarnings("deprecation")
    private void showRestarting(String message) {
        // Android 8.0 refuses an orientation to a see-through activity (it throws).
        if (Build.VERSION.SDK_INT != Build.VERSION_CODES.O) {
            try {
                setRequestedOrientation(ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE);
            } catch (IllegalStateException e) {
                // As it is, then.
            }
        }
        // The game says why it starts again (the world's mods, the launcher...); else, generic.
        setContentView(new RestartView(this,
                message != null && !message.isEmpty() ? message : "Starting Starling again…",
                restartLook(this)));
        View decor = getWindow().getDecorView();
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
            WindowInsetsController bars = decor.getWindowInsetsController();
            if (bars != null) {
                bars.hide(WindowInsets.Type.systemBars());
                bars.setSystemBarsBehavior(
                        WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else {
            decor.setSystemUiVisibility(View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                    | View.SYSTEM_UI_FLAG_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
    }

    /**
     * The launcher's colours as the game kept them before asking for the restart (android.rs
     * `keep_restart_look`): background, plate, plate border, accent, text, muted text and bar
     * track. Horizon Android's default dark theme when there are none.
     */
    private static int[] restartLook(Context context) {
        int[] look = {0xFF13111C, 0xFF1E1A2B, 0xFF5A5468, 0xFF8B78F0, 0xFFF5F2FC, 0xFFA39BBF,
                0xFF2C2839};
        File file = new File(context.getFilesDir(), "restart-look");
        try (FileInputStream in = new FileInputStream(file)) {
            byte[] bytes = new byte[256];
            int n = in.read(bytes);
            String[] parts = new String(bytes, 0, Math.max(n, 0), "UTF-8").trim().split(" ");
            if (parts.length == look.length) {
                int[] read = new int[look.length];
                for (int i = 0; i < read.length; i++) {
                    read[i] = (int) Long.parseLong(parts[i], 16);
                }
                return read;
            }
        } catch (Exception e) {
            // The default look, then.
        }
        return look;
    }

    /**
     * The restart screen: Horizon Android's launcher intro as it ends (android_control_theme.rs
     * `draw_horizon_intro`, its first frame in the new process), the bar moving until the game
     * is back, and the game's line under it.
     */
    private static final class RestartView extends View {
        private final String message;
        private final int[] look;
        private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final RectF rect = new RectF();
        private final long start = SystemClock.uptimeMillis();

        RestartView(Context context, String message, int[] look) {
            super(context);
            this.message = message;
            this.look = look;
        }

        private static int alpha(int color, float a) {
            return (Math.round((color >>> 24) * a) << 24) | (color & 0x00FFFFFF);
        }

        private void rounded(float x, float y, float w, float h, float r, int color) {
            paint.setStyle(Paint.Style.FILL);
            paint.setColor(color);
            rect.set(x, y, x + w, y + h);
            canvas.drawRoundRect(rect, r, r, paint);
        }

        private void stroke(float x, float y, float w, float h, float r, float width, int color) {
            paint.setStyle(Paint.Style.STROKE);
            paint.setStrokeWidth(width);
            paint.setColor(color);
            rect.set(x, y, x + w, y + h);
            canvas.drawRoundRect(rect, r, r, paint);
        }

        /** Text centred on `cx` with its top at `top`, as the launcher's `text_centered`. */
        private void text(String s, float cx, float top, float size, Typeface face, int color) {
            paint.setStyle(Paint.Style.FILL);
            paint.setTypeface(face);
            paint.setTextSize(size);
            paint.setTextAlign(Paint.Align.CENTER);
            paint.setColor(color);
            canvas.drawText(s, cx, top - paint.ascent(), paint);
        }

        private Canvas canvas;

        @Override
        protected void onDraw(Canvas canvas) {
            this.canvas = canvas;
            int bg = look[0], plate = look[1], plateBorder = look[2], accent = look[3];
            int textColor = look[4], muted = look[5], track = look[6];
            float width = getWidth(), height = getHeight();
            // The launcher's units: its screen is about 450 of them tall in landscape.
            float u = Math.max(height / 450f, 1f);
            float w = width / u, h = height / u;
            canvas.drawColor(bg);

            float cx = w * 0.5f;
            float cy = Math.max(110f, Math.min(h * 0.37f, h - 150f));
            float size = Math.max(64f, Math.min(h * 0.19f, 88f));
            // The mark: a ring, a plate, two pillars and the tilted bridge between them.
            float ring = size * 0.64f;
            rounded((cx - ring) * u, (cy - ring) * u, ring * 2 * u, ring * 2 * u, ring * 0.36f * u,
                    alpha(accent, 0.14f));
            stroke((cx - ring) * u, (cy - ring) * u, ring * 2 * u, ring * 2 * u, ring * 0.36f * u,
                    1.5f * u, alpha(accent, 0.38f));
            float plateR = size * 0.46f;
            rounded((cx - plateR) * u, (cy - plateR) * u, plateR * 2 * u, plateR * 2 * u,
                    plateR * 0.32f * u, alpha(plate, 0.94f));
            stroke((cx - plateR) * u, (cy - plateR) * u, plateR * 2 * u, plateR * 2 * u,
                    plateR * 0.32f * u, 1.4f * u, alpha(plateBorder, 0.85f));
            float barW = Math.max(size * 0.135f, 3f), barH = Math.max(size * 0.56f, 10f);
            float offset = size * 0.185f;
            rounded((cx - offset - barW / 2) * u, (cy - barH / 2) * u, barW * u, barH * u,
                    barW / 2 * u, textColor);
            rounded((cx + offset - barW / 2) * u, (cy - barH / 2) * u, barW * u, barH * u,
                    barW / 2 * u, accent);
            paint.setStyle(Paint.Style.STROKE);
            paint.setStrokeCap(Paint.Cap.ROUND);
            paint.setStrokeWidth(Math.max(barW * 0.92f, 2.5f) * u);
            paint.setColor(accent);
            canvas.drawLine((cx - offset) * u, (cy + size * 0.065f) * u, (cx + offset) * u,
                    (cy - size * 0.065f) * u, paint);

            float titleY = cy + size * 0.68f;
            text("STARLING", cx * u, titleY * u, 24f * u,
                    Typeface.create("sans-serif-medium", Typeface.BOLD), textColor);
            float subY = titleY + 32f;
            text("RUST EDITION  •  ANDROID LAUNCHER", cx * u, subY * u, 11.5f * u,
                    Typeface.create("sans-serif-medium", Typeface.NORMAL), alpha(muted, 0.9f));
            // Android is switching processes, so there is no percentage to report.
            float t = (SystemClock.uptimeMillis() - start) / 1000f;
            float bw = Math.max(180f, Math.min(w * 0.34f, 260f)), bh = 6f;
            float bx = cx - bw / 2, by = subY + 28f;
            rounded(bx * u, by * u, bw * u, bh * u, 3f * u, track);
            float segment = bw * 0.30f;
            float phase = 0.5f - 0.5f * (float) Math.cos(t * Math.PI);
            rounded((bx + (bw - segment) * phase) * u, by * u, segment * u, bh * u,
                    3f * u, accent);
            text(message, cx * u, (by + 12f) * u, 11f * u, Typeface.DEFAULT, alpha(muted, 0.88f));
            postInvalidateOnAnimation();
        }
    }

    /** The file an "Open with" or a share hands to Horizon, if any. */
    @SuppressWarnings("deprecation")
    private static Uri handedFile(Intent intent) {
        if (intent == null) {
            return null;
        }
        if (Intent.ACTION_VIEW.equals(intent.getAction())) {
            return intent.getData();
        }
        if (Intent.ACTION_SEND.equals(intent.getAction())) {
            return intent.getParcelableExtra(Intent.EXTRA_STREAM);
        }
        return null;
    }

    /**
     * Copy a file into the game's import inbox as `name.part`, renamed to `name` once complete,
     * on a thread of its own; then `done` on the UI thread (if any).
     */
    static void receive(final Activity activity, final Uri file, final Runnable done) {
        final File inbox = new File(activity.getFilesDir(), "import-inbox");
        final String name = displayName(activity, file);
        new Thread(new Runnable() {
            @Override
            public void run() {
                inbox.mkdirs();
                File part = new File(inbox, name + ".part");
                try (InputStream in = activity.getContentResolver().openInputStream(file);
                        OutputStream out = new FileOutputStream(part)) {
                    if (in == null) {
                        throw new java.io.IOException("nothing to read");
                    }
                    byte[] buffer = new byte[1 << 16];
                    for (int n; (n = in.read(buffer)) > 0; ) {
                        out.write(buffer, 0, n);
                    }
                } catch (Exception e) {
                    part.delete();
                    android.util.Log.w("starling", "could not receive " + name, e);
                }
                if (part.exists() && !part.renameTo(new File(inbox, name))) {
                    part.delete();
                }
                if (done != null) {
                    activity.runOnUiThread(done);
                }
            }
        }, "starling-receive").start();
    }

    /** The file's name as the app it came from shows it, without folders. */
    private static String displayName(Context context, Uri file) {
        String name = null;
        try (Cursor cursor = context.getContentResolver().query(
                file, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                name = cursor.getString(0);
            }
        } catch (Exception e) {
            // A file:// or an odd provider: the path below.
        }
        if (name == null) {
            name = file.getLastPathSegment();
        }
        if (name == null || name.isEmpty()) {
            name = "imported";
        }
        name = name.replaceAll("[/\\\\:*?\"<>|]", "_");
        // ".part" is the inbox's mark for a file still arriving.
        return name.endsWith(".part") ? name + "_" : name;
    }
}
