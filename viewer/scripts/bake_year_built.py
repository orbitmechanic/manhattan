#!/usr/bin/env python3
"""Bake a deterministic construction year into building_perimeters.json.

The shipped dataset is `manhattan_floors.blend` and carries no construction
date or stable OSM id, so a real spatial OSM join is not possible without
guessing ids. Coverage is therefore 0 real / all estimated; the heuristic is
documented here and reproducible:

  base   = 1900 + 110 * clamp(1 - d_km / 6, 0, 1)      (newer near Times Sq)
  bias   = +12 executive, -10 industrial, 0 otherwise
  jitter = (hash(id, block) % 41) - 20 years
  year   = clamp(base + bias + jitter, 1850, 2024)

The field is a generation input only; it never enters the damage journal.
"""
import json, math

TS = (1711.638, -2000.0)  # Times Square projected onto the shipped footprint

def hash01(v):
    v = (v * 2654435761) & 0xFFFFFFFF
    v ^= v >> 16
    v = (v * 2246822519) & 0xFFFFFFFF
    v ^= v >> 13
    v = (v * 3266489917) & 0xFFFFFFFF
    v ^= v >> 16
    return v & 0xFFFFFFFF

def main():
    p = 'assets/models/building_perimeters.json'
    data = json.load(open(p))
    for b in data['buildings']:
        cx, cz = b['centroid']
        d_km = math.hypot(cx - TS[0], cz - TS[1]) / 1000.0
        base = 1900.0 + 110.0 * max(0.0, min(1.0, 1.0 - d_km / 6.0))
        bias = 12 if b['category'] == 'executive' else (-10 if b['category'] == 'industrial' else 0)
        h = hash01(b['id'] * 131 + b['block'] * 17 + 7)
        jitter = (h % 41) - 20
        b['year_built'] = int(max(1850, min(2024, round(base + bias + jitter))))
    data['year_built'] = {
        'real': 0,
        'estimated': len(data['buildings']),
        'method': 'deterministic radial/era heuristic; dataset has no date or stable OSM id',
    }
    json.dump(data, open(p, 'w'), separators=(',', ':'))
    print('baked', len(data['buildings']), 'years')

if __name__ == '__main__':
    main()
