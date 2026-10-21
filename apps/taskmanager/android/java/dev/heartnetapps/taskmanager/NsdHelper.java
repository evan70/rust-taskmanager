package dev.heartnetapps.taskmanager;

import android.app.Activity;
import android.content.Context;
import android.net.nsd.NsdManager;
import android.net.nsd.NsdServiceInfo;

import java.util.ArrayDeque;

/**
 * Finds the phone's own wireless-debugging ports. Android announces them on the local network
 * ("_adb-tls-connect._tcp" always while Wireless debugging is on, "_adb-tls-pairing._tcp" only
 * while the "Pair device with pairing code" window is open), so nobody has to type a port.
 */
public final class NsdHelper {
    private NsdHelper() { }

    private static final String CONNECT = "_adb-tls-connect._tcp";
    private static final String PAIRING = "_adb-tls-pairing._tcp";

    private static volatile int connectPort = 0;
    private static volatile int pairPort = 0;
    private static volatile String connectName = "";
    private static volatile String pairName = "";
    private static NsdManager nsd;
    private static NsdManager.DiscoveryListener connectListener, pairListener;
    private static final ArrayDeque<NsdServiceInfo> queue = new ArrayDeque<NsdServiceInfo>();
    private static boolean resolving = false;

    public static synchronized void start(Activity a) {
        stop();
        try {
            nsd = (NsdManager) a.getApplicationContext().getSystemService(Context.NSD_SERVICE);
            connectListener = listener(true);
            pairListener = listener(false);
            nsd.discoverServices(CONNECT, NsdManager.PROTOCOL_DNS_SD, connectListener);
            nsd.discoverServices(PAIRING, NsdManager.PROTOCOL_DNS_SD, pairListener);
        } catch (Throwable t) {
            nsd = null;
        }
    }

    public static synchronized void stop() {
        try {
            if (nsd != null) {
                if (connectListener != null) nsd.stopServiceDiscovery(connectListener);
                if (pairListener != null) nsd.stopServiceDiscovery(pairListener);
            }
        } catch (Throwable ignored) { }
        nsd = null;
        connectListener = null;
        pairListener = null;
        queue.clear();
        resolving = false;
        connectPort = 0;
        pairPort = 0;
    }

    /** "connect=PORT TAB pair=PORT" with 0 for not found. */
    public static String state() {
        return "connect\t" + connectPort + "\npair\t" + pairPort + "\n";
    }

    private static NsdManager.DiscoveryListener listener(final boolean connect) {
        return new NsdManager.DiscoveryListener() {
            @Override public void onStartDiscoveryFailed(String type, int code) { }
            @Override public void onStopDiscoveryFailed(String type, int code) { }
            @Override public void onDiscoveryStarted(String type) { }
            @Override public void onDiscoveryStopped(String type) { }

            @Override public void onServiceFound(NsdServiceInfo info) {
                enqueue(info, connect);
            }

            @Override public void onServiceLost(NsdServiceInfo info) {
                if (connect && info.getServiceName().equals(connectName)) connectPort = 0;
                if (!connect && info.getServiceName().equals(pairName)) pairPort = 0;
            }
        };
    }

    private static synchronized void enqueue(NsdServiceInfo info, boolean connect) {
        // remember which kind it is in the service type that comes back; resolving is one at a time
        queue.add(info);
        next();
    }

    private static synchronized void next() {
        if (resolving || queue.isEmpty() || nsd == null) return;
        final NsdServiceInfo info = queue.poll();
        resolving = true;
        try {
            nsd.resolveService(info, new NsdManager.ResolveListener() {
                @Override public void onResolveFailed(NsdServiceInfo i, int code) {
                    done();
                }

                @Override public void onServiceResolved(NsdServiceInfo i) {
                    String type = i.getServiceType() == null ? "" : i.getServiceType();
                    if (type.contains("adb-tls-pairing")) {
                        pairName = i.getServiceName();
                        pairPort = i.getPort();
                    } else if (type.contains("adb-tls-connect")) {
                        connectName = i.getServiceName();
                        connectPort = i.getPort();
                    }
                    done();
                }
            });
        } catch (Throwable t) {
            resolving = false;
        }
    }

    private static synchronized void done() {
        resolving = false;
        next();
    }
}
