#!/usr/bin/env python3
"""CityDataset v1.1 adapters. Manhattan is data, not code assumptions."""
import json, math, sys

TS = [1711.638, -2000.0]

def manhattan():
    src = json.load(open('assets/models/building_perimeters.json'))
    buildings = []
    for b in src['buildings']:
        cx, cz = b['centroid']
        buildings.append({
            'id': b['id'], 'block': b['block'],
            'centroid': [cx, cz],
            'perimeter': b['loop'],
            'height_m': b['height_m'],
            'floor_height_m': b['floor_height_m'],
            'category': b['category'],
            'year_built': b.get('year_built', 0),
            'base_elevation_m': 0.0,
        })
    dataset = {
        'version': '1.1',
        'city_id': 'manhattan',
        'attribution': 'manhattan_floors.blend; construction years estimated (see scripts/bake_year_built.py)',
        'license': 'project data',
        'fields': {
            'epicenter': TS,
            'ad_radius_m': 2500.0,
            'ad_full_m': 250.0,
            'glass_radius_m': 900.0,
            'times_square_proxy_note': 'dataset stops near 40.731N; epicenter is the nearest mapped point',
        },
        'terrain': None,
        'water_bodies': None,
        'buildings': buildings,
    }
    json.dump(dataset, open('assets/datasets/manhattan_v11.json','w'), separators=(',',':'))
    print('manhattan records', len(buildings))

def synthetic():
    # invented ten-building city; no Manhattan coupling
    buildings=[]
    for i in range(10):
        x=i*40.0; z=(i%3)*30.0
        w=10+(i%4)*2; d=8+(i%3)*2
        buildings.append({'id':i,'block':i//5,'centroid':[x,z],
          'perimeter':[[x-w/2,z-d/2],[x+w/2,z-d/2],[x+w/2,z+d/2],[x-w/2,z+d/2]],
          'height_m':12+(i%5)*9,'floor_height_m':3.2,'category':'residential' if i%2 else 'industrial',
          'year_built':1900+i*11,'base_elevation_m':0.0})
    json.dump({'version':'1.1','city_id':'synthetic','attribution':'hand-written test','license':'test',
      'fields':{'epicenter':[150.0,30.0]},'terrain':None,'water_bodies':None,'buildings':buildings},
      open('assets/datasets/synthetic_v11.json','w'),separators=(',',':'))
    print('synthetic records', len(buildings))

if __name__=='__main__':
    {'manhattan':manhattan,'synthetic':synthetic}[sys.argv[1]]()
