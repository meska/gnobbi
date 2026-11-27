package net.meskatech.gnobbi;

import static org.junit.Assert.*;

import android.content.Context;
import android.net.Uri;
import androidx.test.core.app.ApplicationProvider;
import java.io.File;
import java.nio.file.Files;
import java.util.Arrays;
import java.util.List;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;
import org.robolectric.annotation.Config;

@RunWith(RobolectricTestRunner.class)
@Config(sdk = 35)
public class PickedFilesTest {
    private final Context ctx = ApplicationProvider.getApplicationContext();

    private Uri source(String name, int size) throws Exception {
        File dir = new File(ctx.getCacheDir(), "source");
        assertTrue(dir.mkdirs() || dir.isDirectory());
        File f = new File(dir, name);
        byte[] bytes = new byte[size];
        Arrays.fill(bytes, (byte) 7);
        Files.write(f.toPath(), bytes);
        return Uri.fromFile(f);
    }

    @Test public void everyItemIsCopiedUnderItsNameInOrderAndTheOverLimitOneStaysEmpty() throws Exception {
        File root = new File(ctx.getCacheDir(), "picked");
        List<Uri> uris = Arrays.asList(source("a.jpg", 10), source("b.jpg", 20), source("scheda.pdf", 300), Uri.parse("file:///nonexistent/gone.png"));
        String[] paths = MainActivity.copyPicked(ctx.getContentResolver(), root, uris, 100);
        assertEquals(4, paths.length);
        assertEquals("a.jpg", new File(paths[0]).getName());
        assertEquals(10, new File(paths[0]).length());
        assertEquals("b.jpg", new File(paths[1]).getName());
        assertEquals(20, new File(paths[1]).length());
        assertEquals("scheda.pdf", new File(paths[2]).getName());
        assertEquals("over the limit: empty, so the preview reports it", 0, new File(paths[2]).length());
        assertEquals("gone.png", new File(paths[3]).getName());
        assertEquals(0, new File(paths[3]).length());
        // The next pick clears the previous copies.
        String[] next = MainActivity.copyPicked(ctx.getContentResolver(), root, uris.subList(0, 1), 100);
        assertFalse(new File(paths[1]).exists());
        assertTrue(new File(next[0]).exists());
    }

    @Test public void aProviderNameNeverEscapesItsFolder() {
        assertEquals("file", MainActivity.displayName(ctx.getContentResolver(), Uri.parse("file:///..")));
        assertEquals("passwd", MainActivity.displayName(ctx.getContentResolver(), Uri.parse("file:///etc/passwd")));
    }
}
