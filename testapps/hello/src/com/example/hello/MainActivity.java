package com.example.hello;

import android.app.Activity;
import android.graphics.Color;
import android.os.Bundle;
import android.util.Log;
import android.widget.TextView;

/** Minimal Activity for apk-patch decode/build and inject-goauld smoke tests. */
public class MainActivity extends Activity {
    private static final String TAG = "apk-patch-hello";

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        TextView tv = new TextView(this);
        tv.setText("apk-patch hello");
        tv.setTextSize(20f);
        tv.setTextColor(Color.WHITE);
        tv.setBackgroundColor(Color.rgb(0x1a, 0x1a, 0x2e));
        tv.setPadding(48, 96, 48, 48);
        setContentView(tv);
        Log.i(TAG, "onCreate");
    }
}
