package com.joey.recordplayer;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import java.io.File;

/**
 * Runs when the tablet finishes booting. The Rust app creates a file called
 * "autostart" in its private folder when "Open on startup" is switched on in
 * Settings (and deletes it when switched off). If the file is there, open the app.
 */
public class BootReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context context, Intent intent) {
        String action = intent.getAction();
        if (!Intent.ACTION_BOOT_COMPLETED.equals(action)
                && !"android.intent.action.QUICKBOOT_POWERON".equals(action)) {
            return;
        }
        File marker = new File(context.getFilesDir(), "autostart");
        if (!marker.exists()) {
            return;
        }
        Intent launch = context.getPackageManager().getLaunchIntentForPackage(context.getPackageName());
        if (launch != null) {
            launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            context.startActivity(launch);
        }
    }
}
