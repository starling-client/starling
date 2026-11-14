package io.github.juanluci.starling;

import android.app.Activity;
import android.app.Dialog;
import android.content.DialogInterface;
import android.content.ClipData;
import android.content.Intent;
import android.net.Uri;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.content.res.ColorStateList;
import android.view.Gravity;
import android.widget.FrameLayout;
import android.os.Bundle;
import android.view.View;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import java.io.File;
import java.util.ArrayList;

/** A system window in its own process. Opening it does not load the game or create a GPU. */
public final class DiagnosticsActivity extends Activity {
    private TextView status;
    private final ArrayList<Button> actions = new ArrayList<Button>();
    private String latest;
    private boolean recovery;
    private int intro;
    private Typeface regular, semibold;
    private int color(int id) { return getResources().getColor(id, getTheme()); }
    private int dp(float value) { return (int) (value * getResources().getDisplayMetrics().density + .5f); }
    private GradientDrawable shape(int fill, int radius, boolean border) {
        GradientDrawable drawable = new GradientDrawable();
        drawable.setColor(fill);
        drawable.setCornerRadius(dp(radius));
        if (border) drawable.setStroke(dp(1), color(R.color.starling_rule));
        return drawable;
    }
    private LinearLayout row(LinearLayout parent) {
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        parent.addView(row);
        return row;
    }
    private native String makeReport(String privateFolder, String gameFolder, boolean export)
            throws java.io.IOException;

    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        recovery = getIntent().getBooleanExtra(StartupRecovery.RECOVERY, false);
        intro = recovery ? R.string.diagnostics_recovery_intro : R.string.diagnostics_intro;
        regular = Typeface.createFromAsset(getAssets(), "diagnostic-fonts/Geist-Regular.ttf");
        semibold = Typeface.createFromAsset(getAssets(), "diagnostic-fonts/Geist-SemiBold.ttf");
        FrameLayout backdrop = new FrameLayout(this);
        backdrop.setBackgroundColor(color(R.color.starling_page));
        backdrop.setPadding(dp(24), dp(24), dp(24), dp(24));
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        int pad = dp(24);
        root.setPadding(pad, pad, pad, pad);
        root.setBackground(shape(color(R.color.starling_page), 14, true));
        TextView title = new TextView(this);
        title.setText(R.string.diagnostics_title);
        title.setTextSize(18);
        title.setTextColor(color(R.color.starling_ink));
        title.setTypeface(semibold);
        root.addView(title);
        status = new TextView(this);
        status.setTextSize(14);
        status.setTextColor(color(R.color.starling_ink_soft));
        status.setTypeface(regular);
        status.setPadding(0, dp(18), 0, dp(18));
        status.setText(intro);
        root.addView(status);
        View.OnClickListener retry = new View.OnClickListener() {
            @Override public void onClick(View view) {
                startActivity(new Intent(DiagnosticsActivity.this, GameActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
                finish();
            }
        };
        if (recovery) add(row(root), R.string.diagnostics_start, retry);
        LinearLayout reports = row(root);
        add(reports, R.string.diagnostics_view_log, new View.OnClickListener() {
            @Override public void onClick(View view) { report(false); }
        });
        add(reports, R.string.diagnostics_export, new View.OnClickListener() {
            @Override public void onClick(View view) { report(true); }
        });
        LinearLayout launching = row(root);
        add(launching, R.string.diagnostics_share, new View.OnClickListener() {
            @Override public void onClick(View view) { share(); }
        });
        if (!recovery) add(launching, R.string.diagnostics_start, retry);
        LinearLayout drivers = row(root);
        add(drivers, R.string.diagnostics_reset_gpu_driver, new View.OnClickListener() {
            @Override public void onClick(View view) { resetGpuDriver(); }
        });
        add(row(root), R.string.diagnostics_close, new View.OnClickListener() {
            @Override public void onClick(View view) { finish(); }
        });
        ScrollView page = new ScrollView(this);
        page.addView(root);
        int available = getResources().getDisplayMetrics().widthPixels - dp(48);
        FrameLayout.LayoutParams card = new FrameLayout.LayoutParams(Math.min(dp(520), available), -2, Gravity.CENTER);
        backdrop.addView(page, card);
        setContentView(backdrop);
        if (recovery) StartupRecovery.offered(this);
        File[] files = new File(getFilesDir(), "performance").listFiles();
        if (files != null) for (File file : files) {
            if (file.isFile() && file.getName().matches("starling-diagnostics-[0-9]+\\.zip")
                    && (latest == null || latest.compareTo(file.getName()) < 0)) latest = file.getName();
        }
    }
    private Button add(LinearLayout root, int text, View.OnClickListener listener) {
        Button button = new Button(this);
        button.setText(text);
        button.setAllCaps(false);
        button.setTextSize(15);
        button.setTypeface(semibold);
        button.setMinHeight(dp(40));
        button.setMinimumHeight(dp(40));
        button.setPadding(dp(12), dp(8), dp(12), dp(8));
        boolean primary = text == (recovery ? R.string.diagnostics_start : R.string.diagnostics_export);
        int ink = color(primary ? R.color.starling_on_accent : R.color.starling_ink);
        button.setTextColor(new ColorStateList(new int[][] {new int[] {-android.R.attr.state_enabled}, new int[] {}}, new int[] {(ink & 0xffffff) | 0x66000000, ink}));
        int fill = primary ? color(R.color.starling_accent) : ((color(R.color.starling_ink) & 0xffffff) | 0x14000000);
        button.setBackground(new RippleDrawable(ColorStateList.valueOf(0x24ecedf3), shape(fill, 10, false), null));
        button.setOnClickListener(listener);
        actions.add(button);
        LinearLayout.LayoutParams layout = new LinearLayout.LayoutParams(0, -2, 1);
        layout.setMargins(0, dp(6), root.getChildCount() == 0 ? dp(6) : 0, dp(6));
        root.addView(button, layout);
        return button;
    }
    private void busy(boolean value) {
        for (Button button : actions) button.setEnabled(!value);
    }
    private void report(final boolean export) {
        busy(true);
        status.setText(R.string.diagnostics_working);
        final String privateFolder = getFilesDir().getAbsolutePath();
        String shared = StarlingActivity.sharedFolder(this);
        final String game = shared == null ? privateFolder : shared;
        new Thread(new Runnable() {
            @Override public void run() {
                try {
                    // Lazy: even a broken native library cannot prevent the system window opening.
                    System.loadLibrary("horizon_android");
                    final String result = makeReport(privateFolder, game, export);
                    runOnUiThread(new Runnable() {
                        @Override public void run() {
                            if (isFinishing() || isDestroyed()) return;
                            busy(false);
                            if (export) {
                                latest = result;
                                status.setText(R.string.diagnostics_exported);
                            } else {
                                showLog(result);
                                status.setText(intro);
                            }
                        }
                    });
                } catch (final Throwable error) {
                    runOnUiThread(new Runnable() {
                        @Override public void run() {
                            if (isFinishing() || isDestroyed()) return;
                            busy(false);
                            status.setText(getString(R.string.diagnostics_failed, String.valueOf(error.getMessage())));
                        }
                    });
                }
            }
        }, "diagnostic-export").start();
    }
    private void showLog(String result) {
        final Dialog dialog = new Dialog(this, android.R.style.Theme_Black_NoTitleBar_Fullscreen);
        FrameLayout backdrop = new FrameLayout(this);
        backdrop.setBackgroundColor(color(R.color.starling_page));
        backdrop.setPadding(dp(24), dp(24), dp(24), dp(24));
        LinearLayout card = new LinearLayout(this);
        card.setOrientation(LinearLayout.VERTICAL);
        card.setPadding(dp(24), dp(24), dp(24), dp(24));
        card.setBackground(shape(color(R.color.starling_page), 14, true));
        TextView title = new TextView(this);
        title.setText(R.string.diagnostics_view_log);
        title.setTextSize(18);
        title.setTypeface(semibold);
        title.setTextColor(color(R.color.starling_ink));
        card.addView(title);
        TextView text = new TextView(this);
        text.setTextIsSelectable(true);
        text.setText(result);
        text.setTypeface(Typeface.MONOSPACE);
        text.setTextSize(12);
        text.setTextColor(color(R.color.starling_ink_soft));
        text.setPadding(0, dp(18), 0, dp(18));
        ScrollView scroll = new ScrollView(this);
        scroll.addView(text);
        card.addView(scroll, new LinearLayout.LayoutParams(-1, 0, 1));
        final Button close = add(row(card), R.string.diagnostics_close, new View.OnClickListener() {
            @Override public void onClick(View view) { dialog.dismiss(); }
        });
        dialog.setOnDismissListener(new DialogInterface.OnDismissListener() {
            @Override public void onDismiss(DialogInterface ignored) { actions.remove(close); }
        });
        int width = Math.min(dp(900), getResources().getDisplayMetrics().widthPixels - dp(48));
        int height = Math.min(dp(600), getResources().getDisplayMetrics().heightPixels - dp(48));
        backdrop.addView(card, new FrameLayout.LayoutParams(width, height, Gravity.CENTER));
        dialog.setContentView(backdrop);
        dialog.show();
    }
    private void share() {
        if (latest == null) { status.setText(R.string.diagnostics_no_report); return; }
        Uri uri = Uri.parse("content://" + getPackageName() + ".updates/diagnostics/" + latest);
        Intent send = new Intent(Intent.ACTION_SEND).setType("application/zip");
        send.putExtra(Intent.EXTRA_STREAM, uri);
        send.setClipData(ClipData.newRawUri(getString(R.string.diagnostics_title), uri));
        send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        try { startActivity(Intent.createChooser(send, null)); }
        catch (android.content.ActivityNotFoundException error) {
            status.setText(getString(R.string.diagnostics_failed, String.valueOf(error.getMessage())));
        }
    }
    /** Back to the system GPU driver without starting the game: forget the chosen custom
     *  driver and its crash count. The installed drivers stay for the next pick. */
    private void resetGpuDriver() {
        File drivers = new File(getFilesDir(), "gpu-drivers");
        boolean reset = new File(drivers, "current").delete() | new File(drivers, "starts").delete();
        status.setText(reset ? R.string.diagnostics_gpu_driver_reset : R.string.diagnostics_gpu_driver_system);
    }
}
