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
# Scenario 4: Exact duplicates — same content as s1, different filenames
# Create 6 duplicates to test batch dedup with larger sample
# ---------------------------------------------------------------------------
# Use 'z_' prefix so duplicates come after 'apple_with_exif.jpg' alphabetically
# This ensures apple_with_exif.jpg is imported first as the canonical file
dup_names = [
    "z_dup_apple_1.jpg",
    "z_dup_apple_2.jpg", 
    "z_dup_apple_3.jpg",
    "z_dup_apple_4.jpg",
    "z_dup_apple_5.jpg",
    "z_dup_apple_6.jpg",
]
for i, dup_name in enumerate(dup_names):
    p_dup = SOURCE_DIR / dup_name
    shutil.copy2(SOURCE_DIR / "apple_with_exif.jpg", p_dup)
    rules.append({
        "id": f"s4_duplicate_{i}" if i > 0 else "s4_duplicate",
        "src": dup_name,
        "scenario": f"exact duplicate #{i+1} of apple_with_exif.jpg" if i > 0 else "exact byte-for-byte duplicate of apple_with_exif.jpg",
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
# Scenario 7 & 8: Filename conflict — same name, different content (collision rename)
# ---------------------------------------------------------------------------
# Create two directories to simulate two cameras with same model
# Both have a file named DSC0001.jpg (camera default naming)
# Need different GPS coordinates to ensure different CRC (CRC is computed from EXIF)

def embed_exif_with_gps(path: Path, dt_original: str, make: str, model: str, 
                        lat: float, lon: float) -> None:
    """Embed EXIF metadata with GPS coordinates."""
    cmd = ["exiftool", "-overwrite_original", "-ignoreMinorErrors"]
    
    cmd.extend([f"-DateTimeOriginal={dt_original}", f"-DateTime={dt_original}"])
    cmd.extend([f"-Make={make}", f"-Model={model}"])
    
    # GPS coordinates
    cmd.extend([
        f"-GPSLatitude={abs(lat)}",
        f"-GPSLatitudeRef={'N' if lat >= 0 else 'S'}",
        f"-GPSLongitude={abs(lon)}",
        f"-GPSLongitudeRef={'E' if lon >= 0 else 'W'}",
    ])
    
    cmd.append(str(path))
    subprocess.run(cmd, check=True, capture_output=True)

# ---------------------------------------------------------------------------
# Scenarios 7-14: Filename conflict stress test - 8 cameras with same filename
# All have DSC0001.jpg (camera default naming), same model, same day
# Different GPS coordinates ensure different CRC/content
# ---------------------------------------------------------------------------

cameras = [
    # (camera_id, gps_location, pixel_rgb, time_offset)
    ("camera_a", (35.6762, 139.6503), (100, 100, 100), "10:00:00"),   # Tokyo
    ("camera_b", (51.5074, -0.1278), (110, 110, 110), "10:05:00"),    # London
    ("camera_c", (40.7128, -74.0060), (120, 120, 120), "10:10:00"),   # New York
    ("camera_d", (48.8566, 2.3522), (130, 130, 130), "10:15:00"),     # Paris
    ("camera_e", (55.7558, 37.6173), (140, 140, 140), "10:20:00"),    # Moscow
    ("camera_f", (39.9042, 116.4074), (150, 150, 150), "10:25:00"),   # Beijing
    ("camera_g", (37.7749, -122.4194), (160, 160, 160), "10:30:00"),  # San Francisco
    ("camera_h", (-33.8688, 151.2093), (170, 170, 170), "10:35:00"),  # Sydney
]

for i, (cam_id, (lat, lon), rgb, time_str) in enumerate(cameras):
    (SOURCE_DIR / cam_id).mkdir(exist_ok=True)
    p = SOURCE_DIR / cam_id / "DSC0001.jpg"
    make_jpeg(p, pixel_rgb=rgb)
    embed_exif_with_gps(p, dt_original=f"2024:05:03 {time_str}", 
                        make="Sony", model="A7IV", lat=lat, lon=lon)
    
    if i == 0:
        # First camera - should get original name
        rules.append({
            "id": f"s7_{cam_id}_first",
            "src": f"{cam_id}/DSC0001.jpg",
            "scenario": f"{cam_id} DSC0001.jpg - first, no rename",
            "expected_status": "imported",
            "expected_dest_contains": ["2024", "05-03", "Sony A7IV"],
            "expected_crc32c_nonnull": True,
            "expected_db_row": True,
            "check_original_name": "DSC0001.jpg",
        })
    else:
        # Subsequent cameras - should be renamed
        rules.append({
            "id": f"s{7+i}_{cam_id}_conflict",
            "src": f"{cam_id}/DSC0001.jpg",
            "scenario": f"{cam_id} DSC0001.jpg - conflict #{i}, should be renamed",
            "expected_status": "imported",
            "expected_dest_contains": ["2024", "05-03", "Sony A7IV"],
            "expected_dest_not_contains": [cam_id],
            "expected_crc32c_nonnull": True,
            "expected_db_row": True,
            "check_renamed_from": "DSC0001.jpg",
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

