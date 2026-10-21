package dev.heartnetapps.taskmanager;

import android.app.Activity;
import android.app.ActivityManager;
import android.app.AppOpsManager;
import android.app.usage.StorageStats;
import android.app.usage.StorageStatsManager;
import android.app.usage.UsageEvents;
import android.app.usage.UsageStats;
import android.app.usage.UsageStatsManager;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;
import android.content.pm.ResolveInfo;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.drawable.Drawable;
import android.net.Uri;
import android.os.BatteryManager;
import android.os.Build;
import android.os.Environment;
import android.os.PowerManager;
import android.os.Process;
import android.os.StatFs;
import android.os.SystemClock;
import android.os.storage.StorageManager;
import android.net.TrafficStats;
import android.provider.Settings;

import java.util.ArrayList;
import java.util.Calendar;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * What Task Manager asks Android for, all through ordinary public APIs: no root, no adb, no network.
 * Text comes back as "key TAB value" lines or one line per app, which the Rust side reads.
 */
public final class SysInfo {
    private SysInfo() { }

    private static void kv(StringBuilder sb, String k, Object v) {
        sb.append(k).append('\t').append(v).append('\n');
    }

    /** One reading of everything that is cheap to ask for. */
    public static String sample(Activity a) {
        StringBuilder sb = new StringBuilder();
        try {
            ActivityManager am = (ActivityManager) a.getSystemService(Context.ACTIVITY_SERVICE);
            ActivityManager.MemoryInfo mi = new ActivityManager.MemoryInfo();
            am.getMemoryInfo(mi);
            kv(sb, "mem_total", mi.totalMem);
            kv(sb, "mem_avail", mi.availMem);
            kv(sb, "mem_low", mi.lowMemory ? 1 : 0);
        } catch (Throwable ignored) { }
        try {
            PowerManager pm = (PowerManager) a.getSystemService(Context.POWER_SERVICE);
            kv(sb, "thermal", pm.getCurrentThermalStatus());
            float h = pm.getThermalHeadroom(10);
            if (!Float.isNaN(h) && !Float.isInfinite(h)) kv(sb, "headroom", h);
            kv(sb, "power_save", pm.isPowerSaveMode() ? 1 : 0);
        } catch (Throwable ignored) { }
        try {
            StatFs fs = new StatFs(Environment.getDataDirectory().getPath());
            kv(sb, "storage_total", fs.getTotalBytes());
            kv(sb, "storage_free", fs.getAvailableBytes());
        } catch (Throwable ignored) { }
        try {
            long rx = TrafficStats.getTotalRxBytes(), tx = TrafficStats.getTotalTxBytes();
            if (rx != TrafficStats.UNSUPPORTED && tx != TrafficStats.UNSUPPORTED) {
                kv(sb, "net_rx", rx);
                kv(sb, "net_tx", tx);
            }
        } catch (Throwable ignored) { }
        kv(sb, "uptime_ms", SystemClock.elapsedRealtime());
        try {
            Intent b = a.registerReceiver(null, new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
            if (b != null) {
                kv(sb, "bat_level", b.getIntExtra(BatteryManager.EXTRA_LEVEL, -1));
                kv(sb, "bat_scale", b.getIntExtra(BatteryManager.EXTRA_SCALE, 100));
                kv(sb, "bat_status", b.getIntExtra(BatteryManager.EXTRA_STATUS, 1));
                kv(sb, "bat_plugged", b.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0));
                kv(sb, "bat_health", b.getIntExtra(BatteryManager.EXTRA_HEALTH, 1));
                kv(sb, "bat_temp", b.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, 0));
                kv(sb, "bat_voltage", b.getIntExtra(BatteryManager.EXTRA_VOLTAGE, 0));
                int cycles = b.getIntExtra("android.os.extra.CYCLE_COUNT", -1);
                if (cycles >= 0) kv(sb, "bat_cycles", cycles);
            }
            BatteryManager bm = (BatteryManager) a.getSystemService(Context.BATTERY_SERVICE);
            long cur = bm.getLongProperty(BatteryManager.BATTERY_PROPERTY_CURRENT_NOW);
            if (cur != Long.MIN_VALUE && cur != 0) kv(sb, "bat_current_ua", cur);
            long full = bm.computeChargeTimeRemaining();
            if (full > 0) kv(sb, "bat_full_in_ms", full);
        } catch (Throwable ignored) { }
        return sb.toString();
    }

    /** About the phone itself; asked for once. */
    public static String device(Activity a) {
        StringBuilder sb = new StringBuilder();
        kv(sb, "model", Build.MANUFACTURER + " " + Build.MODEL);
        kv(sb, "android", Build.VERSION.RELEASE + " (API " + Build.VERSION.SDK_INT + ")");
        kv(sb, "kernel", System.getProperty("os.version"));
        kv(sb, "abi", Build.SUPPORTED_ABIS.length > 0 ? Build.SUPPORTED_ABIS[0] : "?");
        kv(sb, "cores", Runtime.getRuntime().availableProcessors());
        try {
            kv(sb, "refresh", a.getWindowManager().getDefaultDisplay().getRefreshRate());
        } catch (Throwable ignored) { }
        return sb.toString();
    }

    // ---------- usage access ----------



    // ---------- the apps ----------





    /** Apps that ask Android to wake them when the phone starts: package, label, flags. */
    public static String[] bootApps(Activity a) {
        PackageManager pm = a.getPackageManager();
        Map<String, String> lines = new LinkedHashMap<String, String>();
        String[] actions = { Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_LOCKED_BOOT_COMPLETED };
        for (String action : actions) {
            List<ResolveInfo> found;
            try {
                found = pm.queryBroadcastReceivers(new Intent(action), 0);
            } catch (Throwable t) {
                continue;
            }
            for (ResolveInfo r : found) {
                if (r.activityInfo == null) continue;
                String pkg = r.activityInfo.packageName;
                if (pkg.equals(a.getPackageName()) || lines.containsKey(pkg)) continue;
                int system = (r.activityInfo.applicationInfo.flags & ApplicationInfo.FLAG_SYSTEM) != 0 ? 1 : 0;
                String label = pkg;
                try {
                    label = String.valueOf(pm.getApplicationLabel(r.activityInfo.applicationInfo)).replace('\t', ' ').replace('\n', ' ');
                } catch (Throwable ignored) { }
                lines.put(pkg, pkg + "\t" + label + "\t" + system);
            }
        }
        return lines.values().toArray(new String[0]);
    }


    public static int[] icon(Activity a, String pkg, int px) {
        int[] pixels = new int[px * px];
        try {
            Drawable d = a.getPackageManager().getApplicationIcon(pkg);
            Bitmap b = Bitmap.createBitmap(px, px, Bitmap.Config.ARGB_8888);
            Canvas c = new Canvas(b);
            d.setBounds(0, 0, px, px);
            d.draw(c);
            b.getPixels(pixels, 0, px, 0, 0, px, px);
            b.recycle();
        } catch (Throwable ignored) { }
        return pixels;
    }

    // ---------- things a person can do with an app ----------


    public static void appInfo(Activity a, String pkg) {
        try {
            Intent i = new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:" + pkg));
            i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            a.startActivity(i);
        } catch (Throwable ignored) { }
    }



    public static void batterySettings(Activity a) {
        try {
            Intent i = new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS);
            i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            a.startActivity(i);
        } catch (Throwable ignored) { }
    }
}
