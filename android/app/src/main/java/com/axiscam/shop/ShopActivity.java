package com.axiscam.shop;

import android.Manifest;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import android.view.WindowManager;
import io.github.santuzius.icedandroid.IcedActivity;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;

/** The shop tablet's screen stays on while the app is open: a step or a cut is read from it at arm's length, and the Pi stops a run when no computer has been in control for a minute. */
public class ShopActivity extends IcedActivity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        // The notification that says the link stays up (KeepLink). Refused, the link still stays up, unseen.
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[] {Manifest.permission.POST_NOTIFICATIONS}, 0);
        }
        receive(getIntent());
    }

    /** Started from here, in front: Android lets no app start one from the background. Again on each return costs nothing. */
    @Override
    protected void onResume() {
        super.onResume();
        startForegroundService(new Intent(this, KeepLink.class));
    }

    @Override
    protected void onDestroy() {
        if (isFinishing()) {
            stopService(new Intent(this, KeepLink.class));
        }
        super.onDestroy();
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        receive(intent);
    }

    /** A tablet file opened with AXIS Shop (LIO-453) is copied beside the workspace, where the app picks it up on its next tick (bundle::INBOX in src/bundle.rs). */
    private void receive(Intent intent) {
        if (intent == null || !Intent.ACTION_VIEW.equals(intent.getAction()) || intent.getData() == null) {
            return;
        }
        final Uri uri = intent.getData();
        final File dir = new File(getFilesDir(), "AXIS CAM Studio");
        // Off the UI thread: a file from Drive may still be downloading.
        new Thread(() -> {
            dir.mkdirs();
            File part = new File(dir, "inbox.axisjob.part");
            try (InputStream in = getContentResolver().openInputStream(uri);
                 OutputStream out = new FileOutputStream(part)) {
                byte[] buf = new byte[1 << 16];
                for (int n; (n = in.read(buf)) > 0; ) {
                    out.write(buf, 0, n);
                }
            } catch (Exception e) {
                Log.e("AXIS Shop", "Could not read " + uri, e);
                part.delete();
                return;
            }
            // Whole or not at all: the app never reads half a file.
            part.renameTo(new File(dir, "inbox.axisjob"));
        }).start();
        // Opened again later (Recents), the activity must not take the same file twice.
        intent.setAction(Intent.ACTION_MAIN);
    }
}
