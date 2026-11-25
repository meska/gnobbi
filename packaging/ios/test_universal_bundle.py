#!/usr/bin/env python3
"""Check the shared iPhone/iPad bundle metadata."""
import pathlib
import plistlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
ORIENTATIONS = {"UIInterfaceOrientationPortrait", "UIInterfaceOrientationPortraitUpsideDown", "UIInterfaceOrientationLandscapeLeft", "UIInterfaceOrientationLandscapeRight"}


class UniversalBundleTest(unittest.TestCase):
    def test_app_metadata(self):
        for name in ("Info.plist", "Info-device.plist"):
            with self.subTest(name=name), (HERE / name).open("rb") as f:
                info = plistlib.load(f)
                self.assertEqual(info["UIDeviceFamily"], [1, 2])
                self.assertEqual(set(info["UISupportedInterfaceOrientations~ipad"]), ORIENTATIONS)
                self.assertIs(info["UIRequiresFullScreen"], False)
                self.assertIsInstance(info["UILaunchScreen"], dict)

    def test_extension_matches_app_families(self):
        with (HERE / "nse/Info.plist").open("rb") as f:
            self.assertEqual(plistlib.load(f)["UIDeviceFamily"], [1, 2])


if __name__ == "__main__":
    unittest.main()
