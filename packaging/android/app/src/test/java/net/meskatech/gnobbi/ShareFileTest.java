package net.meskatech.gnobbi;

import static org.junit.Assert.*;

import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.provider.OpenableColumns;
import androidx.test.core.app.ApplicationProvider;
import java.io.File;
import java.nio.file.Files;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;
import org.robolectric.Shadows;
import org.robolectric.annotation.Config;

@RunWith(RobolectricTestRunner.class)
@Config(sdk = 35)
public class ShareFileTest {
    private final Context ctx = ApplicationProvider.getApplicationContext();

    private File file(String folder) throws Exception {
        File dir = new File(ctx.getFilesDir(), ".local/share/gnobbi/acct/" + folder);
        assertTrue(dir.mkdirs() || dir.isDirectory());
        File f = new File(dir, "4321-0.pdf");
        Files.write(f.toPath(), new byte[] {1, 2, 3});
        return f;
    }

    @Test public void tempFileGoesToTheChooserUnderTheAttachmentName() throws Exception {
        assertTrue(MainActivity.share(ctx, file("video-tmp").getPath(), "relazione-trimestrale.pdf"));
        Intent chooser = Shadows.shadowOf((android.app.Application) ctx).getNextStartedActivity();
        assertEquals(Intent.ACTION_CHOOSER, chooser.getAction());
        Intent send = chooser.getParcelableExtra(Intent.EXTRA_INTENT, Intent.class);
        assertEquals(Intent.ACTION_SEND, send.getAction());
        assertEquals("application/pdf", send.getType());
        assertTrue((send.getFlags() & Intent.FLAG_GRANT_READ_URI_PERMISSION) != 0);
        Uri uri = send.getParcelableExtra(Intent.EXTRA_STREAM, Uri.class);
        assertEquals("net.meskatech.gnobbi.files", uri.getAuthority());
        try (Cursor c = ctx.getContentResolver().query(uri, null, null, null, null)) {
            assertTrue(c.moveToFirst());
            assertEquals("relazione-trimestrale.pdf", c.getString(c.getColumnIndexOrThrow(OpenableColumns.DISPLAY_NAME)));
        }
    }

    @Test public void anythingOutsideTheTempFolderIsRefused() throws Exception {
        assertFalse(MainActivity.share(ctx, file("store").getPath(), "store.pdf"));
        assertFalse(MainActivity.share(ctx, file("video-tmp").getPath() + "/../../store/4321-0.pdf", "x.pdf"));
        assertNull(Shadows.shadowOf((android.app.Application) ctx).getNextStartedActivity());
    }
}
