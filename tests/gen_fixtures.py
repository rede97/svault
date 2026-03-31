#!/usr/bin/env python3
"""Generate synthetic JPEG test fixtures for svault import pipeline testing.

Usage:
    python3 tests/gen_fixtures.py

Outputs:
    tests/fixtures/source/          - source images to import
    tests/fixtures/chaos/           - files for AI agent chaos scenarios
    tests/fixtures/test_rules.json  - expected outcomes for each scenario
"""
import json
import os
import shutil
import subprocess
import time
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).parent
SOURCE_DIR = ROOT / "fixtures" / "source"
CHAOS_DIR = ROOT / "fixtures" / "chaos"
RULES_FILE = ROOT / "fixtures" / "test_rules.json"

SOURCE_DIR.mkdir(parents=True, exist_ok=True)
CHAOS_DIR.mkdir(parents=True, exist_ok=True)


def make_jpeg(path: Path, pixel_rgb=(128, 64, 32), size=(4, 4)) -> None:
    """Create a minimal JPEG with a solid color."""
    img = Image.new("RGB", size, color=pixel_rgb)
    img.save(path, format="JPEG", quality=85)


def embed_exif(path: Path, dt_original: str = None, make: str = None, model: str = None) -> None:
    """Embed EXIF metadata using exiftool (more reliable than piexif).
    
    piexif can produce IFD chains that the `exif` Rust crate rejects.
    exiftool generates properly formatted EXIF data.
    """
    cmd = ["exiftool", "-overwrite_original", "-ignoreMinorErrors"]
    
    if dt_original:
        # Format: "2024:05:01 10:30:00"
        cmd.extend([f"-DateTimeOriginal={dt_original}", f"-DateTime={dt_original}"])
    if make:
        cmd.extend([f"-Make={make}"])
    if model:
        cmd.extend([f"-Model={model}"])
    
    cmd.append(str(path))
    subprocess.run(cmd, check=True, capture_output=True)


def exif_ts(dt_str: str) -> float:
    """Convert EXIF datetime string to Unix timestamp."""
    return time.mktime(time.strptime(dt_str, "%Y:%m:%d %H:%M:%S"))


rules = []

# ---------------------------------------------------------------------------
# Scenario 1: Normal import — EXIF date + Apple device
# ---------------------------------------------------------------------------
p = SOURCE_DIR / "apple_with_exif.jpg"
make_jpeg(p, pixel_rgb=(200, 100, 50))
embed_exif(p, dt_original="2024:05:01 10:30:00", make="Apple", model="iPhone 15")
rules.append({
    "id": "s1_normal_apple",
    "src": "apple_with_exif.jpg",
    "scenario": "normal import with EXIF date and Apple device",
    "expected_status": "imported",
    "expected_dest_contains": ["2024", "05-01", "Apple iPhone 15", "apple_with_exif.jpg"],
    "expected_crc32c_nonnull": True,
    "expected_db_row": True,
})

# ---------------------------------------------------------------------------
# Scenario 2: Normal import — EXIF date, no device (Unknown fallback)
# ---------------------------------------------------------------------------
p = SOURCE_DIR / "no_device_exif.jpg"
make_jpeg(p, pixel_rgb=(50, 150, 200))
embed_exif(p, dt_original="2024:05:01 18:00:00")
rules.append({
    "id": "s2_no_device",
    "src": "no_device_exif.jpg",
    "scenario": "EXIF date present, no Make/Model → device=Unknown",
    "expected_status": "imported",
    "expected_dest_contains": ["2024", "05-01", "Unknown", "no_device_exif.jpg"],
    "expected_crc32c_nonnull": True,
    "expected_db_row": True,
})

# ---------------------------------------------------------------------------
# Scenario 3: No EXIF at all — path derived from mtime
# ---------------------------------------------------------------------------
p = SOURCE_DIR / "no_exif.jpg"
make_jpeg(p, pixel_rgb=(10, 200, 10))
# Set mtime to 2024-03-15 08:00:00
target_ts = exif_ts("2024:03:15 08:00:00")
os.utime(p, (target_ts, target_ts))
rules.append({
    "id": "s3_no_exif",
    "src": "no_exif.jpg",
    "scenario": "no EXIF — dest path uses mtime fallback",
    "expected_status": "imported",
    "expected_dest_contains": ["2024", "03-15", "Unknown", "no_exif.jpg"],
    "expected_crc32c_nonnull": True,
    "expected_db_row": True,
})

# ---------------------------------------------------------------------------
# Scenario 4: Exact duplicate — same content as s1, different filename
# ---------------------------------------------------------------------------
p_dup = SOURCE_DIR / "duplicate_of_apple.jpg"
shutil.copy2(SOURCE_DIR / "apple_with_exif.jpg", p_dup)
rules.append({
    "id": "s4_duplicate",
    "src": "duplicate_of_apple.jpg",
    "scenario": "exact byte-for-byte duplicate of apple_with_exif.jpg",
    "expected_status": "duplicate",
    "expected_dup_reason": ["db", "batch"],
    "expected_db_row": False,
    "note": "Must be imported AFTER apple_with_exif.jpg to trigger DB dedup",
})

# ---------------------------------------------------------------------------
# Scenario 5: Samsung device
# ---------------------------------------------------------------------------
p = SOURCE_DIR / "samsung_photo.jpg"
make_jpeg(p, pixel_rgb=(30, 80, 180))
embed_exif(p, dt_original="2024:05:02 14:20:00", make="Samsung", model="Galaxy S24")
rules.append({
    "id": "s5_samsung",
    "src": "samsung_photo.jpg",
    "scenario": "Samsung device — model already starts with 'Samsung', no duplication",
    "expected_status": "imported",
    "expected_dest_contains": ["2024", "05-02", "Samsung", "samsung_photo.jpg"],
    "expected_crc32c_nonnull": True,
    "expected_db_row": True,
})

# ---------------------------------------------------------------------------
# Scenario 6: Model string that starts with Make (avoid "Apple Apple iPhone")
# ---------------------------------------------------------------------------
p = SOURCE_DIR / "apple_redundant_model.jpg"
make_jpeg(p, pixel_rgb=(180, 90, 20))
embed_exif(p, dt_original="2024:05:02 09:00:00", make="Apple", model="Apple iPhone 14")
rules.append({
    "id": "s6_make_in_model",
    "src": "apple_redundant_model.jpg",
    "scenario": "Model starts with Make — device should be 'Apple iPhone 14', not 'Apple Apple iPhone 14'",
    "expected_status": "imported",
    "expected_dest_contains": ["Apple iPhone 14"],
    "expected_dest_not_contains": ["Apple Apple"],
    "expected_crc32c_nonnull": True,
    "expected_db_row": True,
})

# ---------------------------------------------------------------------------
# Chaos fixtures — for AI agent scenarios
# ---------------------------------------------------------------------------

# chaos/renamed_after_import.jpg  — same content as s1, simulates user renaming
p_chaos = CHAOS_DIR / "renamed_after_import.jpg"
shutil.copy2(SOURCE_DIR / "apple_with_exif.jpg", p_chaos)

# chaos/moved_subdirectory/file.jpg — same content as s2, simulates user moving
(CHAOS_DIR / "moved_subdirectory").mkdir(exist_ok=True)
shutil.copy2(SOURCE_DIR / "no_device_exif.jpg", CHAOS_DIR / "moved_subdirectory" / "no_device_exif.jpg")

# chaos/interrupted_copy.jpg — a truncated/corrupt JPEG (simulates interrupted copy)
p_trunc = CHAOS_DIR / "interrupted_copy.jpg"
make_jpeg(p_trunc, pixel_rgb=(255, 0, 0))
with open(p_trunc, "r+b") as f:
    f.seek(0, 2)
    size = f.tell()
    f.truncate(size // 2)  # truncate to half — invalid JPEG

# chaos/extra_copy.jpg — another unique image for "add then delete" scenario
p_extra = CHAOS_DIR / "extra_unique.jpg"
make_jpeg(p_extra, pixel_rgb=(0, 0, 255))
embed_exif(p_extra, dt_original="2023:11:06 12:00:00", make="Google", model="Pixel 8")
# Remove EXIF from other chaos files that shouldn't have it
for chaos_file in ["interrupted_copy.jpg"]:
    p = CHAOS_DIR / chaos_file
    if p.exists():
        subprocess.run(["exiftool", "-overwrite_original", "-all=", str(p)], 
                      check=False, capture_output=True)

# ---------------------------------------------------------------------------
# Write test_rules.json
# ---------------------------------------------------------------------------
output = {
    "version": 1,
    "description": "Expected outcomes for svault import pipeline tests",
    "path_template": "$year/$mon-$day/$device/$filename",
    "scenarios": rules,
    "chaos_scenarios": [
        {
            "id": "c1_rename_before_import",
            "action": "Rename chaos/renamed_after_import.jpg to a new name before importing",
            "expected": "Still detected as duplicate (same CRC32C + strong hash), not re-imported",
        },
        {
            "id": "c2_move_to_subdir",
            "action": "Move file into subdirectory within source, then import",
            "expected": "Still found by VFS walk, imported correctly",
        },
        {
            "id": "c3_interrupt_copy",
            "action": "Import a truncated/corrupt JPEG",
            "expected": "Hash error logged, status=failed, file NOT written to DB",
        },
        {
            "id": "c4_add_delete_mid_import",
            "action": "Agent adds/deletes files from source dir while import runs",
            "expected": "Import completes without crash; missing files counted as failed",
        },
        {
            "id": "c5_repeat_import",
            "action": "Run import twice on the same source directory",
            "expected": "Second run: all files classified as LikelyCacheDuplicate, imported=0",
        },
    ],
}

with open(RULES_FILE, "w") as f:
    json.dump(output, f, indent=2, ensure_ascii=False)

print(f"Generated {len(list(SOURCE_DIR.glob('*.jpg')))} source fixtures")
print(f"Generated {len(list(CHAOS_DIR.rglob('*')))} chaos fixtures")
print(f"Rules written to {RULES_FILE}")

