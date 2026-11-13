package com.axiscam.shop;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.os.PowerManager;

/**
 * Keeps the link to the Pi up while the owner is in another app or the screen is off (LIO-462).
 * Without it Android cuts a background app's sockets within seconds, and a minute later the Pi stops the run.
 * A foreground service, shown as a notification, keeps the app's network; the wake lock keeps the link's thread running with the screen off.
 * It runs while AXIS Shop is open, and ends with it.
 */
public class KeepLink extends Service {
    private static final String CHANNEL = "link";
    private PowerManager.WakeLock wake;

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        NotificationManager nm = getSystemService(NotificationManager.class);
        nm.createNotificationChannel(new NotificationChannel(CHANNEL, "Link to the Pi", NotificationManager.IMPORTANCE_LOW));
        PendingIntent back = PendingIntent.getActivity(this, 0,
                new Intent(this, ShopActivity.class).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE);
        Notification n = new Notification.Builder(this, CHANNEL)
                .setSmallIcon(android.R.drawable.stat_notify_sync)
                .setContentTitle("AXIS Shop is linked to the Pi")
                .setContentText("A run goes on while you use other apps. Tap to go back.")
                .setContentIntent(back)
                .setOngoing(true)
                .build();
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(1, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE);
        } else {
            startForeground(1, n);
        }
        if (wake == null) {
            wake = getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "AXIS Shop:link");
            wake.acquire();
        }
        return START_NOT_STICKY;
    }

    @Override
    public void onDestroy() {
        if (wake != null) {
            wake.release();
        }
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
