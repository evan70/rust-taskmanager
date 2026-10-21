package dev.heartnetapps.taskmanager;

import android.app.RemoteInput;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;

/**
 * Receives the pairing code typed into the notification. The notification lets the code be entered
 * without leaving the Settings screen that shows it (Android closes that window if you switch away).
 */
public class PairReceiver extends BroadcastReceiver {
    /** Implemented in Rust (registered at start-up). */
    static native void nativeCode(String code);

    public static final String ACTION = "dev.heartnetapps.taskmanager.PAIR_CODE";

    @Override public void onReceive(Context context, Intent intent) {
        Bundle results = RemoteInput.getResultsFromIntent(intent);
        CharSequence code = results == null ? null : results.getCharSequence("code");
        if (code == null) return;
        try {
            nativeCode(code.toString().trim());
        } catch (Throwable t) {
            // the app was not running, so there is nothing to hand the code to
            AdvHelper.notifyPlain(context, "Task Manager", "Open Task Manager, tap Set up detailed mode again, then enter the code.");
        }
    }
}
