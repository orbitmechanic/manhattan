#!/usr/bin/env python3
"""S1.5 Python verification: matcher samples, provenance accounting, binary
partition accounting, and horizon packing contracts.

Run from ``viewer/``: python3 scripts/test_s1b.py
"""

from __future__ import annotations

import json
import math
import os
import unittest

import numpy as np
from shapely.geometry import Point, Polygon

ROOT = "datasets/seattle"
ALLOWED = {
    "real_surveyed",
    "lidar_overture",
    "osm_levels_derived",
    "msml_estimate",
    "area_heuristic",
}


def load(path):
    with open(path) as handle:
        return json.load(handle)


class Data:
    enriched = None

    @classmethod
    def dataset(cls):
        if cls.enriched is None:
            cls.enriched = load(f"{ROOT}/seattle_v11_enriched.json")
        return cls.enriched


class ProvenanceTest(unittest.TestCase):
    def test_every_building_has_a_contracted_source(self):
        buildings = Data.dataset()["buildings"]
        self.assertEqual(len(buildings), 330_330)
        seen = set()
        for building in buildings:
            source = building.get("height_source")
            self.assertIn(source, ALLOWED, source)
            seen.add(source)
            self.assertGreater(building["height_m"], 0.0)
        self.assertEqual(seen, ALLOWED)

    def test_report_accounting_sums(self):
        report = load(f"{ROOT}/height_report.json")
        self.assertEqual(sum(report["height_source_counts"].values()), 330_330)
        for city, counts in report["after"].items():
            self.assertEqual(sum(counts.values()), sum(report["baseline"][city].values()))
        seattle = report["after"]["seattle"]
        covered = seattle["real_surveyed"] + seattle["lidar_overture"] + seattle.get(
            "osm_levels_derived", 0
        )
        self.assertGreaterEqual(covered / sum(seattle.values()), 0.95)
        bellevue = report["after"]["bellevue"]
        self.assertGreater(bellevue["lidar_overture"], 40_000)

    def test_matcher_samples_recompute(self):
        envelope = load("assets/datasets/seattle_enrich_samples.json")
        self.assertGreaterEqual(len(envelope["samples"]), 10)
        by_id = {b["id"]: b for b in Data.dataset()["buildings"]}
        for sample in envelope["samples"]:
            original = Polygon(sample["original_perimeter"])
            overture = Polygon(sample["overture_perimeter"])
            self.assertTrue(original.is_valid)
            self.assertTrue(overture.is_valid)
            self.assertTrue(overture.contains(original.centroid))
            ratio = original.area / max(overture.area, 1e-6)
            self.assertGreaterEqual(ratio, 0.40 - 1e-6)
            self.assertLessEqual(ratio, 2.50 + 1e-6)
            after = by_id[sample["building_id"]]
            if after["height_source"] == "lidar_overture" and sample["overture_height_m"]:
                self.assertAlmostEqual(
                    after["height_m"], sample["overture_height_m"], delta=0.01
                )


class PartitionTest(unittest.TestCase):
    def test_partition_accounting_and_size_win(self):
        report = load(f"{ROOT}/partition_report.json")
        nodes = load(f"{ROOT}/partitions.json")["nodes"]
        neighborhoods = [row for row in nodes if row["level"] == 2]
        self.assertEqual(len(neighborhoods), report["levels"]["neighborhoods"])
        self.assertEqual(sum(row["buildings"] for row in neighborhoods), 330_330)
        self.assertEqual(sum(row["streets"] for row in neighborhoods), 29_833)
        self.assertLess(report["binary_bytes"], report["json_bytes"] / 4.0)
        self.assertLess(
            report["single_partition_bytes"],
            report["single_partition_json_bytes"],
        )
        for row in neighborhoods:
            self.assertGreater(row["buildings"], 0)
            self.assertEqual(len(row["center"]), 2)
            self.assertEqual(len(row["aabb"]), 4)
            self.assertIn("dominant_category", row["hints"])
            self.assertIn("dominant_source", row["hints"])

    def test_hierarchy_paths_and_containment(self):
        nodes = load(f"{ROOT}/partitions.json")["nodes"]
        counties = [row for row in nodes if row["level"] == 0]
        cities = [row for row in nodes if row["level"] == 1]
        neighborhoods = [row for row in nodes if row["level"] == 2]
        self.assertEqual(len(counties), 2)
        self.assertEqual(len(cities), 3)
        self.assertEqual(len(neighborhoods), 1096)
        self.assertEqual({row["name"] for row in counties}, {"King", "Pierce"})
        counts = {row["name"]: row["buildings"] for row in cities}
        self.assertEqual(counts, {"Seattle": 187_053, "Bellevue": 56_856, "Tacoma": 86_421})
        for row in neighborhoods:
            parent = nodes[row["parent"]]
            self.assertEqual(parent["level"], 1)
            a, b = row["aabb"], parent["aabb"]
            self.assertGreaterEqual(a[0], b[0] - 0.011)
            self.assertGreaterEqual(a[1], b[1] - 0.011)
            self.assertLessEqual(a[2], b[2] + 0.011)
            self.assertLessEqual(a[3], b[3] + 0.011)

    def test_evaluation_report_and_tacoma_gate(self):
        report = load(f"{ROOT}/eval_report.json")
        poses = {row["pose"]: row for row in report["poses"]}
        self.assertEqual(report["flat"], 1096)
        self.assertGreater(poses["downtown"]["ratio_10x"], 10.0)
        far = poses["far-south"]
        self.assertEqual(far["flat"], 1096)
        self.assertEqual(far["hierarchical_10x"]["cities"], 0)
        self.assertEqual(far["hierarchical_10x"]["neighborhoods"], 0)
        self.assertGreater(far["ratio_10x"], 100.0)

    def test_pack_file_and_fixture_exist(self):
        self.assertGreater(os.path.getsize("assets/datasets/seattle.citypack"), 10_000_000)
        self.assertLess(os.path.getsize("assets/datasets/seattle.citypack"), 30_000_000)
        expected = load("assets/datasets/seattle_test_expected.json")
        self.assertEqual(expected["nodes"], 3)
        self.assertGreater(expected["buildings"], 0)
        self.assertEqual(len(expected["samples"]), 12)


class GeologyTest(unittest.TestCase):
    def test_s2_assets_and_audits(self):
        report = load(f"{ROOT}/s2_report.json")
        self.assertEqual(report["grid"]["rows"], 2322)
        self.assertEqual(report["grid"]["cols"], 1207)
        self.assertEqual(report["grid"]["spacing_m"], 30.0)
        self.assertEqual(os.path.getsize("assets/terrain/seattle_dem_30m.bin"), 2322 * 1207 * 2)
        self.assertGreater(os.path.getsize("assets/terrain/seattle_water_mask.bin"), 2_000_000)
        self.assertGreater(os.path.getsize("assets/terrain/seattle_water_mask.bin"), 2_700_000)
        self.assertGreater(os.path.getsize("assets/terrain/seattle_water.bin"), 300_000)
        self.assertEqual(report["streets"]["segments"], 29_833)
        self.assertEqual(report["streets"]["max_drape_error_m"], 0.0)
        self.assertEqual(report["lod"]["seam_max_height_delta_m"], 0.0)
        self.assertGreater(report["water"]["bodies"], 100)
        self.assertGreater(report["water"]["triangles"], 10_000)
        self.assertGreater(report["water"]["mask_cells"], 400_000)
        levels = report["water"]["surface_levels"]
        self.assertIn(0.0, levels)
        self.assertTrue(any(abs(level - 4.98) < 0.01 for level in levels))

    def test_s2_perf_table(self):
        rows = load(f"{ROOT}/s2_perf.json")
        self.assertGreaterEqual(len(rows), 14)
        names = {row["pose"] for row in rows}
        for required in ("gauntlet_650ms", "low_orbit", "chunk_hover"):
            self.assertIn(required, names)
        for row in rows:
            self.assertGreater(row["avg_ms"], 0.0)
            self.assertGreaterEqual(row["p99_ms"], row["avg_ms"] - 0.01)
            self.assertGreater(row["samples"], 1)

    def test_dem_spot_elevations(self):
        import numpy as np

        report = load(f"{ROOT}/s2_report.json")
        grid = np.fromfile("assets/terrain/seattle_dem_30m.bin", dtype="<i2").reshape(
            report["grid"]["rows"], report["grid"]["cols"]
        ).astype(np.float32) / 10.0
        origin = report["grid"]["origin_world"]
        spacing = report["grid"]["spacing_m"]

        def sample(x, z):
            col = int(round((x - origin[0]) / spacing - 0.5))
            row = int(round((z - origin[1]) / spacing - 0.5))
            return float(grid[row, col])

        for x, z, expected in [
            (0.0, 0.0, 30.6),
            (1224.3, -1866.7, 103.1),
            (-1548.5, -3455.5, 117.9),
        ]:
            self.assertLess(abs(sample(x, z) - expected), 2.0)


class HorizonTest(unittest.TestCase):
    GRID = 400

    def test_packed_size_and_contract_cells(self):
        grid_path = "assets/horizon/seattle_horizon.bin"
        profiles_path = "assets/horizon/seattle_profiles.bin"
        self.assertEqual(os.path.getsize(grid_path), 400 * 400 * 2)
        self.assertEqual(os.path.getsize(profiles_path), 360 * 400 * 2)
        grid = np.fromfile(grid_path, dtype="<u2").reshape(400, 400)
        report = load(f"{ROOT}/horizon_report.json")

        def decode(cell):
            packed = int(grid[cell])
            return (packed >> 2) * 0.5, bool(packed & 0b10), bool(packed & 0b01)

        elevation, tree, snow = decode(tuple(report["test_cells"]["rainier"][:2]))
        self.assertAlmostEqual(elevation, 4393.0, delta=3.0)
        self.assertTrue(snow)
        self.assertFalse(tree)
        forest_cell = tuple(report["test_cells"]["forest"][:2])
        _elevation, tree, _snow = decode(forest_cell)
        self.assertTrue(tree)
        offshore = tuple(report["test_cells"]["offshore"][:2])
        elevation, tree, snow = decode(offshore)
        self.assertEqual(elevation, 0.0)
        self.assertFalse(tree)
        self.assertFalse(snow)
        self.assertLessEqual(os.path.getsize(grid_path), 320_000)
        self.assertLessEqual(os.path.getsize(profiles_path), 320_000)

    def test_rainier_quadrant_and_angle(self):
        report = load(f"{ROOT}/horizon_report.json")
        rainier = report["rainier"]
        self.assertTrue(140.0 <= rainier["bearing_deg"] <= 165.0)
        self.assertAlmostEqual(rainier["distance_km"], 94.2, delta=2.0)
        angle = math.degrees(math.atan2(rainier["elevation_m"] - 250.0, rainier["distance_km"] * 1000.0))
        self.assertAlmostEqual(angle, 2.52, delta=0.2)

    def test_profiles_step_and_shape(self):
        profiles = np.fromfile("assets/horizon/seattle_profiles.bin", dtype="<u2").reshape(360, 400)
        self.assertEqual(profiles.shape, (360, 400))
        distances = (np.arange(400) + 1) * 500.0
        angles = np.degrees(np.arctan2(profiles * 0.5 - 250.0, distances))
        near = angles[:, distances <= 15_000.0].max(axis=1)
        mid = angles[:, distances <= 90_000.0].max(axis=1)
        far = np.where(distances > 90_000.0, angles, -90.0).max(axis=1)
        far = np.maximum(far, mid)
        apex = int(np.argmax(far))
        self.assertLessEqual(abs(apex - 152), 2)
        self.assertGreater(far[apex], mid[apex] + 0.2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
