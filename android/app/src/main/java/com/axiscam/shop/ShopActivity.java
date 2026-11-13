package com.axiscam.shop;

import android.os.Bundle;
import android.view.WindowManager;
import io.github.santuzius.icedandroid.IcedActivity;

/** The shop tablet's screen stays on while the app is open: a step or a cut is read from it at arm's length, and the Pi stops a run when no computer has been in control for a minute. */
public class ShopActivity extends IcedActivity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
    }
}
