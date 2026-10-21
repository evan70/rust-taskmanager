package dev.heartnetapps.taskmanager;

import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.RemoteInput;
import android.content.Context;
import android.content.Intent;
import android.graphics.drawable.Icon;
import android.os.Build;

import java.io.File;

/** Small things the detailed mode needs from Android: folders, and the notification that takes the pairing code. */
public final class AdvHelper {
    private AdvHelper() { }

    private static final String CHANNEL = "pairing";
    private static final int NOTE_ID = 4711;

    /** Where the app's native libraries were unpacked (the helper program is among them). */
    public static String nativeLibDir(Activity a) {
        return a.getApplicationInfo().nativeLibraryDir;
    }

    /** The app's folder on shared storage; the phone's shell can read it, other apps cannot. Created if needed. */
    public static String externalFilesDir(Activity a) {
        File d = new File("/sdcard/Android/data/" + a.getPackageName() + "/files");
        d.mkdirs();
        return d.getAbsolutePath();
    }

    public static boolean canNotify(Activity a) {
        return Build.VERSION.SDK_INT < 33 || a.checkSelfPermission("android.permission.POST_NOTIFICATIONS") == android.content.pm.PackageManager.PERMISSION_GRANTED;
    }

    public static void requestNotifyPermission(Activity a) {
        if (Build.VERSION.SDK_INT >= 33) {
            a.requestPermissions(new String[] { "android.permission.POST_NOTIFICATIONS" }, 11);
        }
    }

    private static void ensureChannel(Context c) {
        NotificationManager nm = (NotificationManager) c.getSystemService(Context.NOTIFICATION_SERVICE);
        nm.createNotificationChannel(new NotificationChannel(CHANNEL, "Setting up detailed mode", NotificationManager.IMPORTANCE_HIGH));
    }

    private static Icon smallIcon(Context c) {
        int id = c.getResources().getIdentifier("ic_stat", "drawable", c.getPackageName());
        if (id == 0) id = android.R.drawable.ic_dialog_info;
        return Icon.createWithResource(c, id);
    }

    /** The notification shown while waiting for the pairing code, with a box to type it in. */
    public static void showPairing(Activity a, String title, String text, boolean withInput) {
        try {
            Context c = a.getApplicationContext();
            ensureChannel(c);
            Notification.Builder b = new Notification.Builder(c, CHANNEL)
                    .setSmallIcon(smallIcon(c))
                    .setContentTitle(title)
                    .setContentText(text)
                    .setStyle(new Notification.BigTextStyle().bigText(text))
                    .setOnlyAlertOnce(true)
                    .setOngoing(false);
            if (withInput) {
                RemoteInput ri = new RemoteInput.Builder("code").setLabel("Pairing code").build();
                Intent i = new Intent(c, PairReceiver.class).setAction(PairReceiver.ACTION).setPackage(c.getPackageName());
                PendingIntent pi = PendingIntent.getBroadcast(c, 0, i, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_MUTABLE);
                Notification.Action act = new Notification.Action.Builder(smallIcon(c), "Enter pairing code", pi).addRemoteInput(ri).build();
                b.addAction(act);
            }
            ((NotificationManager) c.getSystemService(Context.NOTIFICATION_SERVICE)).notify(NOTE_ID, b.build());
        } catch (Throwable ignored) { }
    }

    public static void cancelPairing(Activity a) {
        try {
            ((NotificationManager) a.getSystemService(Context.NOTIFICATION_SERVICE)).cancel(NOTE_ID);
        } catch (Throwable ignored) { }
    }

    /** A plain message, from a place that has no Activity (the receiver). */
    static void notifyPlain(Context c, String title, String text) {
        try {
            ensureChannel(c);
            Notification n = new Notification.Builder(c, CHANNEL).setSmallIcon(smallIcon(c)).setContentTitle(title).setContentText(text).build();
            ((NotificationManager) c.getSystemService(Context.NOTIFICATION_SERVICE)).notify(NOTE_ID, n);
        } catch (Throwable ignored) { }
    }

    /** Opens Developer options, where Wireless debugging is. */
    public static void openDeveloperOptions(Activity a) {
        try {
            Intent i = new Intent(android.provider.Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS);
            i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            a.startActivity(i);
        } catch (Throwable t) {
            try {
                Intent i = new Intent(android.provider.Settings.ACTION_DEVICE_INFO_SETTINGS);
                i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                a.startActivity(i);
            } catch (Throwable ignored) { }
        }
    }
}
