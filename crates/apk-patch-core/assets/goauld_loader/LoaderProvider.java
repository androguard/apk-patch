package goauld.inject;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.net.Uri;
import android.util.Log;

/**
 * Early-load hook for libgoauld_agent.so via System.loadLibrary.
 * Registered in the host APK manifest as a non-exported ContentProvider.
 */
public final class LoaderProvider extends ContentProvider {
    private static final String TAG = "goauld";

    static {
        try {
            System.loadLibrary("goauld_agent");
        } catch (Throwable t) {
            Log.e(TAG, "failed to load goauld_agent", t);
        }
    }

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] selectionArgs, String sortOrder) {
        return null;
    }

    @Override
    public String getType(Uri uri) {
        return null;
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        return null;
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        return 0;
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        return 0;
    }
}
