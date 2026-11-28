package net.meskatech.gnobbi;

import org.junit.runner.RunWith;
import org.robolectric.RobolectricTestRunner;
import org.robolectric.annotation.Config;

/** Same cases on API 27, where the signer comes from GET_SIGNATURES instead of the signing info. */
@RunWith(RobolectricTestRunner.class)
@Config(sdk = 27)
public class SelfUpdateLegacyTest extends SelfUpdateTest {}
