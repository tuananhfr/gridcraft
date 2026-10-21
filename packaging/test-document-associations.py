"""Run with python3 packaging/test-document-associations.py."""
import plistlib
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[1]
EXTENSIONS = {"xlsx", "xlsm", "xlsb", "ods", "csv", "tsv"}


class DocumentAssociations(unittest.TestCase):
    def test_macos_document_icons(self):
        plist = plistlib.loads((ROOT / "packaging/macos/Info.plist.in").read_bytes())
        documents = plist["CFBundleDocumentTypes"]
        self.assertEqual({ext for doc in documents for ext in doc["CFBundleTypeExtensions"]}, EXTENSIONS)
        self.assertEqual(plist["CFBundleIconFile"], "gridcraft")
        for doc in documents:
            with self.subTest(document=doc["CFBundleTypeName"]):
                self.assertEqual(doc["CFBundleTypeIconFile"], plist["CFBundleIconFile"])
                self.assertEqual(doc["CFBundleTypeRole"], "Editor")
                self.assertEqual(doc["LSHandlerRank"], "Alternate")
        script = (ROOT / "packaging/macos/package.sh").read_text()
        self.assertIn('cp "$ROOT/assets/app-icon/gridcraft.icns" "$APP/Contents/Resources/gridcraft.icns"', script)

    def test_windows_candidates_and_icons(self):
        tree = ET.parse(ROOT / "packaging/windows/gridcraft.wxs")
        values = [node.attrib for node in tree.iter("{http://wixtoolset.org/schemas/v4/wxs}RegistryValue")]
        capabilities = "Software\\ERPCons\\BangTinh\\Capabilities"
        associations = {v["Name"]: v["Value"] for v in values if v["Key"] == capabilities + "\\FileAssociations"}
        self.assertEqual(associations, {"." + ext: "ERPCons.BangTinh." + ext for ext in EXTENSIONS})
        for ext in EXTENSIONS:
            prog_id = associations["." + ext]
            expected = [
                ("Software\\Classes\\." + ext + "\\OpenWithProgids", prog_id, prog_id),
                ("Software\\Classes\\" + prog_id + "\\DefaultIcon", None, '"[#GridcraftExe]",0'),
                ("Software\\Classes\\" + prog_id + "\\shell\\open\\command", None, '"[#GridcraftExe]" "%1"'),
            ]
            for key, name, value in expected:
                with self.subTest(extension=ext, key=key):
                    self.assertTrue(any(v["Root"] == "HKLM" and v["Key"] == key and v.get("Name") == name
                                        and v["Value"] == value for v in values))
        self.assertTrue(any(v["Key"] == "Software\\RegisteredApplications" and v["Value"] == capabilities for v in values))
        for value in values:
            self.assertNotIn("userchoice", value["Key"].lower())
            self.assertFalse(value["Key"].startswith("Software\\Classes\\.") and not value.get("Name"))


if __name__ == "__main__":
    unittest.main()
