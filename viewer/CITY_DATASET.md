# CityDataset v1.1

JSON: `version`, `city_id`, `attribution`, `license`, optional `fields`
(epicenter, ad/glass radii), optional `terrain` (elevation grid), optional
`water_bodies` (polygons + datum). `buildings[]`: id, block, centroid, perimeter
(world metres), height_m, floor_height_m, category, optional year_built,
optional base_elevation_m (default 0). Adapters: `scripts/city_dataset.py
manhattan|synthetic`. Pilot generator consumes only this file
(`PILOT_DATASET`), never `building_perimeters.json`.
