package dev.horizon.android;

import android.graphics.Bitmap;
import android.graphics.Color;
import android.media.MediaMetadata;
import android.media.session.MediaController;
import android.media.session.MediaSessionManager;
import android.media.session.PlaybackState;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.service.notification.NotificationListenerService;
import java.util.List;

/**
 * Publishes the active media session for the game's mod API (capability
 * `lyrics`). Reads only the session's metadata — title, artist, album,
 * length, playback position and the cover bitmap — never the contents of
 * notifications.
 *
 * The Rust side calls [#currentSnapshotJson()] about once a second; a null
 * return means nothing is playing.
 */
public final class HorizonNowPlayingService extends NotificationListenerService {
    private static volatile String snapshotJson;

    /** The current session as JSON, or null when none. Called from Rust. */
    public static String currentSnapshotJson() {
        return snapshotJson;
    }

    private MediaSessionManager sessions;
    private android.content.ComponentName component;
    private Handler handler;
    private final Runnable refresh = new Runnable() {
        @Override
        public void run() {
            update();
            if (handler != null) {
                handler.postDelayed(this, 1000);
            }
        }
    };

    @Override
    public void onListenerConnected() {
        sessions = (MediaSessionManager) getSystemService(MEDIA_SESSION_SERVICE);
        component = new android.content.ComponentName(this, HorizonNowPlayingService.class);
        handler = new Handler(Looper.getMainLooper());
        handler.post(refresh);
    }

    @Override
    public void onListenerDisconnected() {
        if (handler != null) {
            handler.removeCallbacks(refresh);
            handler = null;
        }
        snapshotJson = null;
    }

    private void update() {
        if (sessions == null) {
            return;
        }
        try {
            List<MediaController> active = sessions.getActiveSessions(component);
            MediaController chosen = null;
            for (MediaController controller : active) {
                PlaybackState state = controller.getPlaybackState();
                int value = state == null ? PlaybackState.STATE_NONE : state.getState();
                if (value == PlaybackState.STATE_PLAYING || value == PlaybackState.STATE_BUFFERING) {
                    chosen = controller;
                    break;
                }
            }
            if (chosen == null && !active.isEmpty()) {
                // The most recently active paused session.
                chosen = active.get(0);
            }
            snapshotJson = chosen == null ? null : snapshot(chosen);
        } catch (Throwable error) {
            snapshotJson = null;
            android.util.Log.w("HorizonNowPlaying", "media session read failed", error);
        }
    }

    private static String snapshot(MediaController controller) {
        MediaMetadata metadata = controller.getMetadata();
        PlaybackState plays = controller.getPlaybackState();
        boolean playing = false;
        long position = 0;
        long positionAtElapsed = 0;
        if (plays != null) {
            int value = plays.getState();
            playing = value == PlaybackState.STATE_PLAYING || value == PlaybackState.STATE_BUFFERING;
            position = plays.getPosition();
            positionAtElapsed = plays.getLastPositionUpdateTime();
        }
        long nowWall = System.currentTimeMillis();
        long nowElapsed = SystemClock.elapsedRealtime();
        long positionAtWall = nowWall;
        if (positionAtElapsed > 0) {
            positionAtWall = nowWall - (nowElapsed - positionAtElapsed);
            if (positionAtWall < 0) {
                positionAtWall = 0;
            }
        }

        String title = "";
        String artist = "";
        String album = "";
        String app = "";
        long duration = 0;
        Long artColor = null;
        try {
            app = controller.getPackageName();
        } catch (Throwable ignored) {
            app = "";
        }
        if (metadata != null) {
            title = text(metadata.getString(MediaMetadata.METADATA_KEY_TITLE));
            artist = text(metadata.getString(MediaMetadata.METADATA_KEY_ARTIST));
            if (artist.isEmpty()) {
                artist = text(metadata.getString(MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE));
            }
            album = text(metadata.getString(MediaMetadata.METADATA_KEY_ALBUM));
            duration = metadata.getLong(MediaMetadata.METADATA_KEY_DURATION);
            Bitmap art = metadata.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART);
            if (art == null) {
                art = metadata.getBitmap(MediaMetadata.METADATA_KEY_ART);
            }
            if (art != null) {
                artColor = vibrant(art);
            }
        }

        StringBuilder out = new StringBuilder(256);
        out.append("{\"title\":\"").append(escape(title)).append("\",\"artist\":\"").append(escape(artist)).append("\",\"album\":\"").append(escape(album)).append("\",\"duration_ms\":").append(duration).append(",\"position_ms\":").append(position).append(",\"position_at_ms\":").append(positionAtWall).append(",\"now_ms\":").append(nowWall).append(",\"playing\":").append(playing);
        out.append(",\"app\":\"").append(escape(app)).append("\"");
        if (artColor != null) {
            out.append(",\"art_color\":").append(artColor.longValue());
        } else {
            out.append(",\"art_color\":null");
        }
        out.append('}');
        return out.toString();
    }

    private static String text(String value) {
        return value == null ? "" : value;
    }

    private static String escape(String value) {
        StringBuilder out = new StringBuilder(value.length() + 8);
        for (int i = 0; i < value.length(); i++) {
            char c = value.charAt(i);
            switch (c) {
                case '"':
                    out.append("\\\"");
                    break;
                case '\\':
                    out.append("\\\\");
                    break;
                case '\n':
                    out.append("\\n");
                    break;
                case '\r':
                    out.append("\\r");
                    break;
                case '\t':
                    out.append("\\t");
                    break;
                default:
                    if (c < 0x20) {
                        out.append(String.format("\\u%04x", (int) c));
                    } else {
                        out.append(c);
                    }
            }
        }
        return out.toString();
    }

    /** The most saturated bright colour of the cover, or null for grey art. */
    private static Long vibrant(Bitmap source) {
        Bitmap sampled = Bitmap.createScaledBitmap(source, 32, 32, true);
        float bestScore = 0.35f;
        int best = 0;
        float[] hsv = new float[3];
        boolean found = false;
        for (int y = 0; y < sampled.getHeight(); y++) {
            for (int x = 0; x < sampled.getWidth(); x++) {
                int pixel = sampled.getPixel(x, y);
                if (Color.alpha(pixel) < 128) {
                    continue;
                }
                Color.colorToHSV(pixel, hsv);
                float saturation = hsv[1];
                float value = hsv[2];
                if (value < 0.25f || saturation < 0.35f) {
                    continue;
                }
                // Prefer vivid mid-bright colours over near-white and near-black.
                float score = saturation * (1.0f - Math.abs(value - 0.75f));
                if (score > bestScore) {
                    bestScore = score;
                    best = pixel | 0xFF000000;
                    found = true;
                }
            }
        }
        if (sampled != source) {
            sampled.recycle();
        }
        return found ? Long.valueOf(Integer.toUnsignedLong(best)) : null;
    }
}
