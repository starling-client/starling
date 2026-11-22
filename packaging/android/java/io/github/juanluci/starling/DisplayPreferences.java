package io.github.juanluci.starling;

import android.content.pm.ActivityInfo;
import android.content.res.Configuration;
import android.graphics.Point;
import android.graphics.Rect;
import android.os.Build;
import android.view.DisplayCutout;
import android.view.Gravity;
import android.view.OrientationEventListener;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowManager;
import android.view.WindowMetrics;

/** Applies the game's display options on the activity's UI thread. */
@SuppressWarnings("deprecation")
final class DisplayPreferences {
    private final StarlingActivity activity;
    private final OrientationEventListener sensor;
    private final int defaultGravity;
    private int cutout, orientations, rotation, aspectWidth, aspectHeight;
    private int sensorSide = -1;
    private boolean resumed;
    private boolean naturalLandscape;
    private boolean reverseDefaultRotation;
    private int lastDisplayRotation = -1;
    private DisplayCutout api28Cutout;

    DisplayPreferences(StarlingActivity activity) {
        this.activity = activity;
        defaultGravity = activity.getWindow().getAttributes().gravity;
        sensor = new OrientationEventListener(activity) {
            @Override
            public void onOrientationChanged(int degrees) {
                if (degrees == ORIENTATION_UNKNOWN || rotation != 1) return;
                // Hold the last side for 15 degrees past the division between two sides.
                if (sensorSide >= 0 && Math.abs((degrees - sensorSide * 90 + 540) % 360 - 180) < 60) {
                    return;
                }
                int side = ((degrees + 45) / 90) % 4;
                boolean landscape = naturalLandscape != (side % 2 != 0);
                if (orientations != 2 && landscape != (orientations == 0)) return;
                sensorSide = side;
                boolean reversed = side == 2
                        || side % 2 != 0 && ((side == 3) == naturalLandscape) != reverseDefaultRotation;
                request(fixed(landscape, !reversed));
            }
        };
    }

    void set(int cutout, int orientations, int rotation, int width, int height) {
        boolean rotationChanged = this.orientations != orientations || this.rotation != rotation;
        this.cutout = cutout;
        this.orientations = orientations;
        this.rotation = rotation;
        aspectWidth = width;
        aspectHeight = height;
        if (rotationChanged) applyRotation();
        layout();
    }

    private void applyRotation() {
        boolean currentLandscape =
                activity.getResources().getConfiguration().orientation == Configuration.ORIENTATION_LANDSCAPE;
        boolean sideways = (activity.getWindowManager().getDefaultDisplay().getRotation() & 1) != 0;
        naturalLandscape = currentLandscape != sideways;
        int direction = activity.getResources().getIdentifier(
                "config_reverseDefaultRotation", "bool", "android");
        reverseDefaultRotation = direction != 0 && activity.getResources().getBoolean(direction);
        boolean landscape = orientations == 2 ? currentLandscape : orientations == 0;
        if (rotation == 0) {
            request(orientations == 2 ? ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR
                    : landscape ? ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
                    : ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT);
        } else {
            request(fixed(landscape, rotation == 1 || rotation == 3));
        }
        sensorSide = -1;
        listen(resumed);
    }

    private static int fixed(boolean landscape, boolean reverse) {
        if (landscape) return reverse ? ActivityInfo.SCREEN_ORIENTATION_REVERSE_LANDSCAPE
                : ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE;
        return reverse ? ActivityInfo.SCREEN_ORIENTATION_REVERSE_PORTRAIT
                : ActivityInfo.SCREEN_ORIENTATION_PORTRAIT;
    }

    private void request(int mode) {
        if (activity.getRequestedOrientation() != mode) activity.setRequestedOrientation(mode);
    }

    void listen(boolean active) {
        resumed = active;
        if (active && rotation == 1 && sensor.canDetectOrientation()) sensor.enable();
        else sensor.disable();
    }

    void layout() {
        Window window = activity.getWindow();
        WindowManager.LayoutParams attrs = window.getAttributes();
        Rect frame;
        DisplayCutout notch = null;
        if (Build.VERSION.SDK_INT >= 30) {
            WindowMetrics metrics = activity.getWindowManager().getCurrentWindowMetrics();
            frame = metrics.getBounds();
            notch = metrics.getWindowInsets().getDisplayCutout();
        } else {
            Point size = new Point();
            activity.getWindowManager().getDefaultDisplay().getRealSize(size);
            frame = new Rect(0, 0, size.x, size.y);
            if (Build.VERSION.SDK_INT >= 29) {
                notch = activity.getWindowManager().getDefaultDisplay().getCutout();
            } else if (Build.VERSION.SDK_INT == 28) {
                int displayRotation = activity.getWindowManager().getDefaultDisplay().getRotation();
                if (lastDisplayRotation != displayRotation) {
                    lastDisplayRotation = displayRotation;
                    api28Cutout = null;
                }
                WindowInsets insets = window.getDecorView().getRootWindowInsets();
                // Once cropped, the window's insets cease to include the strip it removed.
                if (insets != null && attrs.width == WindowManager.LayoutParams.MATCH_PARENT
                        && attrs.height == WindowManager.LayoutParams.MATCH_PARENT) {
                    api28Cutout = insets.getDisplayCutout();
                }
                notch = api28Cutout;
            }
        }
        int[] safe = notch == null ? new int[4] : new int[] {
                notch.getSafeInsetLeft(), notch.getSafeInsetTop(),
                notch.getSafeInsetRight(), notch.getSafeInsetBottom()
        };
        int width = frame.width(), height = frame.height();
        if (width <= 0 || height <= 0) return;
        Rect area = new Rect(0, 0, width, height);
        if (cutout == 1 && safe[0] + safe[2] < width && safe[1] + safe[3] < height) {
            area.set(safe[0], safe[1], width - safe[2], height - safe[3]);
        }
        if (aspectWidth > 0 && aspectHeight > 0) {
            boolean portrait = height > width;
            int a = portrait ? aspectHeight : aspectWidth;
            int b = portrait ? aspectWidth : aspectHeight;
            int fittedWidth = Math.max(1, Math.min(area.width(), area.height() * a / b));
            int fittedHeight = Math.max(1, Math.min(area.height(), area.width() * b / a));
            int left = area.left + (area.width() - fittedWidth) / 2;
            int top = area.top + (area.height() - fittedHeight) / 2;
            area.set(left, top, left + fittedWidth, top + fittedHeight);
        }
        boolean full = area.width() == width && area.height() == height;
        int gravity = full ? defaultGravity : Gravity.TOP | Gravity.LEFT;
        int windowWidth = full ? WindowManager.LayoutParams.MATCH_PARENT : area.width();
        int windowHeight = full ? WindowManager.LayoutParams.MATCH_PARENT : area.height();
        if (attrs.width != windowWidth || attrs.height != windowHeight || attrs.gravity != gravity
                || attrs.x != area.left || attrs.y != area.top) {
            attrs.width = windowWidth;
            attrs.height = windowHeight;
            attrs.gravity = gravity;
            attrs.x = area.left;
            attrs.y = area.top;
            attrs.dimAmount = full ? 0f : 1f;
            if (full) attrs.flags &= ~WindowManager.LayoutParams.FLAG_DIM_BEHIND;
            else attrs.flags |= WindowManager.LayoutParams.FLAG_DIM_BEHIND;
            window.setAttributes(attrs);
        }
        activity.reportCutout(Math.max(0, safe[0] - area.left), Math.max(0, safe[1] - area.top),
                Math.max(0, safe[2] - (width - area.right)),
                Math.max(0, safe[3] - (height - area.bottom)));
    }
}
