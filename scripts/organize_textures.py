#!/usr/bin/env python3
"""Organise the Manhattan texture dump into day/, night/ and library/ with
descriptive names. Originals are preserved in backup/Textures_original.zip."""
import os, shutil, json
from PIL import Image

ROOT = "/home/orbit9/Work/games/Manhattan/Textures"

if os.path.isdir(os.path.join(ROOT, "day")):
    raise SystemExit("Textures are already organised (day/ exists); this script was a one-off run.")
DAY = os.path.join(ROOT, "day")
NIGHT = os.path.join(ROOT, "night")
LIB = os.path.join(ROOT, "library")

for d in [DAY, NIGHT, LIB,
          os.path.join(DAY, "facades"), os.path.join(DAY, "roofs"),
          os.path.join(DAY, "ground"), os.path.join(DAY, "streets"),
          os.path.join(DAY, "environment"),
          os.path.join(NIGHT, "facades"), os.path.join(NIGHT, "roofs"),
          os.path.join(NIGHT, "ground"), os.path.join(NIGHT, "streets")]:
    os.makedirs(d, exist_ok=True)

# (original, destination relative to Textures)
MOVES = {
    # ---- used by the model: canonical day textures ----
    "TexturesCom_HighRiseResidential0197_2_seamless_S.jpg": "day/facades/facade_residential_beige_0197.jpg",
    "TexturesCom_HighRiseGlass0110_4_seamless_S.jpg": "day/facades/facade_glass_white_0110.jpg",
    "TexturesCom_HighRiseGlass0053_1_seamless_S.jpg": "day/facades/facade_glass_bronze_0053.jpg",
    "TexturesCom_HighRiseGlass0056_1_seamless_S.jpg": "day/facades/facade_glass_blue_0056.jpg",
    "images (21).jpg": "day/facades/facade_glass_skyblue_grid.jpg",
    "TexturesCom_BuildingsHighRise0564_1_S.jpg": "day/facades/facade_glass_striped_0564.jpg",
    "images (4).jpg": "day/facades/facade_glass_dark_vertical.jpg",
    "images (19).jpg": "day/facades/facade_glass_blue_horizontal.jpg",
    "TexturesCom_BuildingsHighRise0607_1_S.jpg": "day/facades/facade_office_white_0607.jpg",
    "images (20).jpg": "day/facades/facade_masonry_beige_classic.jpg",
    "roof5.jpg": "day/roofs/roof_concrete_gravel.jpg",
    # ---- street set (used for night lighting demo / future road meshes) ----
    "road.jpg": "day/streets/road_asphalt.jpg",
    # ---- library: unused but useful assets, renamed descriptively ----
    "4880219806629888.jpg": "library/facade_residential_glass_tower_01.jpg",
    "6293fa4cad13168420de47380d0b0e36.jpg": "library/facade_glass_bronze_wide_01.jpg",
    "8a635e6585d4ceacba00ec627996cdaa.jpg": "library/facade_office_white_grid_01.jpg",
    "Building_texture10.jpg": "library/facade_office_white_10.jpg",
    "Building_texture3.jpg": "library/facade_glass_blue_green_bands_03.jpg",
    "Building_texture5.jpg": "library/facade_office_glass_grid_05.jpg",
    "Building_texture6.jpg": "library/facade_glass_dark_bands_06.jpg",
    "Building_texture8.jpg": "library/facade_glass_blue_mosaic_08.jpg",
    "Building_texture9.jpg": "library/facade_glass_green_grid_09.jpg",
    "BuildingsHighRise0551_1_download600.jpg": "library/facade_masonry_white_windows_0551.jpg",
    "BuildingsHighRise0558_download600.jpg": "library/facade_glass_blue_vertical_0558.jpg",
    "Capture.PNG": "library/signage_neon_teal_lines.png",
    "Facade001_2K_Color.jpg": "library/facade_glass_dark_grid_001.jpg",
    "Facade006_2K_Color.jpg": "library/facade_glass_white_bands_006.jpg",
    "HighRiseGlass0048_1_download600.jpg": "library/facade_glass_pale_blue_0048.jpg",
    "HighRiseResidential0062_1_download600.jpg": "library/facade_residential_brick_0062a.jpg",
    "HighRiseResidential0062_2_download600.jpg": "library/facade_residential_brick_0062b.jpg",
    "TexturesCom_BuildingsHighRise0497_1_seamless_S.jpg": "library/facade_masonry_glass_0497.jpg",
    "TexturesCom_BuildingsHighRise0560_1_S.jpg": "library/facade_glass_blue_stripes_0560.jpg",
    "TexturesCom_BuildingsHighRise0624_2_seamless_S.jpg": "library/facade_masonry_arches_0624.jpg",
    "TexturesCom_BuildingsHighRise0625_2_seamless_S.jpg": "library/facade_office_cream_grid_0625.jpg",
    "TexturesCom_BuildingsHighRise0634_5_seamless_S.jpg": "library/facade_masonry_cream_0634.jpg",
    "TexturesCom_BuildingsHighRise0639_5_seamless_S.jpg": "library/facade_office_concrete_0639.jpg",
    "TexturesCom_Fabric_Camouflage_512_albedo.tif": "library/material_fabric_camouflage.tif",
    "TexturesCom_HighRiseGlass0031_1_seamless_S.jpg": "library/facade_glass_gray_grid_0031.jpg",
    "TexturesCom_HighRiseGlass0048_1_seamless_S.jpg": "library/facade_glass_blue_white_0048.jpg",
    "TexturesCom_HighRiseGlass0082_1_S.jpg": "library/facade_glass_green_vertical_0082.jpg",
    "TexturesCom_HighRiseGlass0086_1_S.jpg": "library/facade_glass_blue_mullion_0086.jpg",
    "TexturesCom_HighRiseGlass0091_1_seamless_S.jpg": "library/facade_glass_blue_band_0091.jpg",
    "TexturesCom_HighRiseGlass0101_3_seamless_S.jpg": "library/facade_glass_gray_bands_0101.jpg",
    "TexturesCom_HighRiseGlass0104_3_seamless_S.jpg": "library/facade_glass_green_bands_0104.jpg",
    "TexturesCom_HighRiseResidential0098_1_seamless_S.jpg": "library/facade_residential_brick_rows_0098.jpg",
    "TexturesCom_HighRiseResidential0101_1_seamless_S.jpg": "library/facade_residential_brick_0101.jpg",
    "TexturesCom_HighRiseTowers0158_3_S.jpg": "library/facade_residential_brick_balcony_0158.jpg",
    "TexturesCom_MetalPlatesBare0158_10_seamless_S.jpg": "library/material_metal_plates.jpg",
    "White bricks.png": "library/material_brick_white.png",
    "aerial_grass_rock_ao_1k.jpg": "library/ground_grass_rock_ao.jpg",
    "aerial_grass_rock_diff_1k.jpg": "library/ground_grass_rock_diffuse.jpg",
    "dark table wood.PNG": "library/material_wood_dark_table.png",
    "download (1).jpg": "library/facade_office_white_stripes_01.jpg",
    "download (10).jpg": "library/facade_glass_white_office_10.jpg",
    "download (11).jpg": "library/facade_glass_pale_grid_11.jpg",
    "download (2).jpg": "library/facade_office_gray_grid_02.jpg",
    "download (3).jpg": "library/facade_masonry_red_03.jpg",
    "download (4).jpg": "library/facade_glass_blue_grid_04.jpg",
    "download (5).jpg": "library/facade_masonry_red_05.jpg",
    "download (6).jpg": "library/facade_office_white_rows_06.jpg",
    "download (7).jpg": "library/facade_glass_white_bands_07.jpg",
    "download (8).jpg": "library/facade_masonry_white_grid_08.jpg",
    "download (9).jpg": "library/facade_office_white_windows_09.jpg",
    "download.jpg": "library/facade_masonry_brick_01.jpg",
    "download4.jpg": "library/facade_glass_blue_vertical_04.jpg",
    "ea2874c813cd1d48138d045f6ca43c7b.jpg": "library/facade_office_gray_glass_02.jpg",
    "f13bf40efcab60e4cdabd13d4c81856a.jpg": "library/facade_glass_blue_bands_02.jpg",
    "grass.jpg": "library/ground_grass.jpg",
    "images (1).jpg": "library/facade_glass_teal_grid_01.jpg",
    "images (10).jpg": "library/facade_glass_dark_grid_10.jpg",
    "images (11).jpg": "library/facade_office_white_rows_11.jpg",
    "images (12).jpg": "library/facade_masonry_red_12.jpg",
    "images (13).jpg": "library/facade_concrete_white_fins_13.jpg",
    "images (14).jpg": "library/facade_glass_blue_14.jpg",
    "images (15).jpg": "library/facade_glass_green_stripes_15.jpg",
    "images (16).jpg": "library/facade_residential_white_16.jpg",
    "images (17).jpg": "library/facade_glass_blue_mullion_17.jpg",
    "images (18).jpg": "library/facade_glass_teal_bands_18.jpg",
    "images (2).jpg": "library/facade_glass_teal_grid_02.jpg",
    "images (22).jpg": "library/facade_masonry_brick_mixed_22.jpg",
    "images (23).jpg": "library/facade_concrete_dark_columns_23.jpg",
    "images (24).jpg": "library/facade_masonry_cream_24.jpg",
    "images (25).jpg": "library/facade_glass_blue_25.jpg",
    "images (26).jpg": "library/facade_masonry_white_arched_26.jpg",
    "images (27).jpg": "library/facade_glass_green_teal_27.jpg",
    "images (3).jpg": "library/facade_office_white_dense_03.jpg",
    "images (5).jpg": "library/facade_office_white_glass_05.jpg",
    "images (6).jpg": "library/facade_glass_blue_06.jpg",
    "images (7).jpg": "library/facade_office_white_grid_07.jpg",
    "images (8).jpg": "library/facade_masonry_beige_08.jpg",
    "images (9).jpg": "library/facade_office_white_bands_09.jpg",
    "images.jpg": "library/facade_glass_teal_columns_01.jpg",
    "militarytexture.jpg": "library/material_camouflage.jpg",
    "peopleColors.png": "library/material_people_palette.png",
    "roo4.jpg": "library/roof_plaster_pink.jpg",
    "roof 1.webp": "library/roof_concrete_white.webp",
    "roof 2.jpg": "library/roof_stucco_pink.jpg",
    "roof3.jpg": "library/roof_stucco_white.jpg",
    "roof6.jpg": "library/roof_metal_panel_gray.jpg",
    "texture 1.jpg": "library/facade_office_white_01.jpg",
    "unnamed (1).jpg": "library/facade_glass_blue_02.jpg",
    "unnamed.jpg": "library/facade_office_gray_03.jpg",
}

manifest = {}
made = []
errors = []
for old, new in MOVES.items():
    src = os.path.join(ROOT, old)
    dst = os.path.join(ROOT, new)
    if not os.path.exists(src):
        errors.append(f"missing {old}")
        continue
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.move(src, dst)
    manifest[old] = new
    made.append(new)

# street textures: convert TIF to JPG for portability
for old, new_day, new_lib in [
    ("TexturesCom_ConcreteSidewalk_2x2_1K_albedo.tif",
     "day/streets/sidewalk_concrete.jpg", "library/source_sidewalk_concrete.tif"),
    ("TexturesCom_Pavement_Weave_1K_roughness.tif",
     "day/streets/pavement_weave_roughness.jpg", "library/source_pavement_weave_roughness.tif"),
]:
    src = os.path.join(ROOT, old)
    if os.path.exists(src):
        im = Image.open(src)
        if im.mode not in ("RGB", "L"):
            im = im.convert("RGB")
        im.save(os.path.join(ROOT, new_day), quality=92, optimize=True)
        manifest[old] = new_day
        # keep the source tif in library for lossless reference
        shutil.move(src, os.path.join(ROOT, new_lib))
        made.append(new_day)

# the aerial grass subfolder: rename to a descriptive folder and its files
sub = os.path.join(ROOT, "aerial_grass_rock_1k_jpg")
subnew = os.path.join(LIB, "ground_grass_rock_1k")
if os.path.isdir(sub):
    os.makedirs(subnew, exist_ok=True)
    for f in os.listdir(sub):
        nf = f.replace("aerial_grass_rock_", "")
        shutil.move(os.path.join(sub, f), os.path.join(subnew, nf))
        manifest[os.path.join("aerial_grass_rock_1k_jpg", f)] = os.path.join(
            "library/ground_grass_rock_1k", nf)
    os.rmdir(sub)

# remove the now-empty source image directories
for d in ["day/ground", "day/environment", "night"]:
    pass

# write manifest
with open(os.path.join(ROOT, "rename_manifest.json"), "w") as f:
    json.dump(manifest, f, indent=2, sort_keys=True)

print(f"moved {len(made)} files")
if errors:
    print("ERRORS:", errors)
print("remaining top-level:", sorted(os.listdir(ROOT)))
for base in [DAY, LIB]:
    for root, dirs, files in os.walk(base):
        print(root.replace(ROOT, "."), len(files))
