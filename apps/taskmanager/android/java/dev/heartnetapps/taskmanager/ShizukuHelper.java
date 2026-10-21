package dev.heartnetapps.taskmanager;

import android.app.Activity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;

import rikka.shizuku.Shizuku;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.lang.reflect.Method;
import java.util.concurrent.TimeUnit;

/**
 * Uses Shizuku, if the person has installed and started it, to run the line that starts the helper
 * (an alternative to the app's own pairing). Everything about Shizuku stays in this class.
 */
public final class ShizukuHelper {
    private ShizukuHelper() { }

    private static final String SHIZUKU = "moe.shizuku.privileged.api";
    public static final int INSTALLED = 1;
    public static final int RUNNING = 2;
    public static final int GRANTED = 4;

    /** Bits: 1 = installed, 2 = running, 4 = this app is allowed to use it. */
    public static int state(Activity a) {
        int s = 0;
        try {
            a.getPackageManager().getPackageInfo(SHIZUKU, 0);
            s |= INSTALLED;
        } catch (Throwable ignored) { }
        try {
            if (Shizuku.pingBinder()) {
                s |= RUNNING;
                if (!Shizuku.isPreV11() && Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED) s |= GRANTED;
            }
        } catch (Throwable ignored) { }
        return s;
    }

    /** Shows Shizuku's own "allow this app?" dialog. */
    public static void requestPermission() {
        try {
            if (Shizuku.pingBinder() && !Shizuku.isPreV11()) Shizuku.requestPermission(5);
        } catch (Throwable ignored) { }
    }

    /** Opens the Shizuku app, or its page in the store when it is not installed. */
    public static void open(Activity a) {
        try {
            Intent i = a.getPackageManager().getLaunchIntentForPackage(SHIZUKU);
            if (i == null) i = new Intent(Intent.ACTION_VIEW, Uri.parse("https://play.google.com/store/apps/details?id=" + SHIZUKU));
            i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            a.startActivity(i);
        } catch (Throwable ignored) { }
    }

    private static byte[] drain(InputStream in) throws Exception {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buf = new byte[4096];
        int n;
        while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
        return out.toByteArray();
    }

    /**
     * Runs a shell line through Shizuku and returns what it printed. The library keeps its
     * process call private, so it is called by reflection (the library is bundled with this app, so
     * its version never changes underneath us). Failures come back as text starting with "ERROR:".
     */
    public static String run(String script, long timeoutMs) {
        try {
            Method m = Shizuku.class.getDeclaredMethod("newProcess", String[].class, String[].class, String.class);
            m.setAccessible(true);
            final Process p = (Process) m.invoke(null, new Object[] { new String[] { "sh", "-c", script }, null, null });
            final byte[][] err = new byte[1][];
            Thread t = new Thread(new Runnable() {
                @Override public void run() {
                    try { err[0] = drain(p.getErrorStream()); } catch (Throwable ignored) { }
                }
            });
            t.start();
            byte[] out = drain(p.getInputStream());
            boolean done = ((rikka.shizuku.ShizukuRemoteProcess) p).waitForTimeout(timeoutMs, TimeUnit.MILLISECONDS);
            t.join(1000);
            String text = new String(out, "UTF-8");
            if (err[0] != null && err[0].length > 0) text += new String(err[0], "UTF-8");
            if (!done) return "ERROR: timed out. " + text;
            return text;
        } catch (Throwable t) {
            return "ERROR: " + t;
        }
    }
}
