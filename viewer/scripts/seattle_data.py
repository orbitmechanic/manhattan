#!/usr/bin/env python3
"""Seattle S1: scripted ArcGIS/3DEP fetch + CityDataset v1.1 adapter."""
import json, os, sys, urllib.parse, urllib.request
import numpy as np
import rasterio
from shapely.geometry import shape
from shapely.strtree import STRtree
from pyproj import Transformer

ROOT='datasets/seattle'; RAW=ROOT+'/raw'
BASE='https://services.arcgis.com/ZOyb2t4B0UYuYNYH/arcgis/rest/services'
# Seattle city envelope in EPSG:2926 (downtown + south + east shore)
ENV='1240000,170000,1325000,270000'
LAYERS={
 'buildings_2023':'Building_Outlines_2023',
 'buildings_2015':'Building_Outline_2015',
 'snd':'Street_Network_Database_SND',
 'zoning':'Current_Land_Use_Zoning_Generalized_new',
 'shoreline':'Shoreline',
}
def get(url):
    with urllib.request.urlopen(url, timeout=60) as r: return json.load(r)
def fetch(name, service):
    feats=[]; off=0
    while True:
        q={'where':'1=1','outFields':'*','returnGeometry':'true','outSR':'2926',
           'f':'geojson','resultOffset':off,'resultRecordCount':2000}
        if name in ('buildings_2023','buildings_2015','snd','zoning'):
            q['geometry']=ENV; q['geometryType']='esriGeometryEnvelope'; q['inSR']='2926'; q['spatialRel']='esriSpatialRelIntersects'
        url=f'{BASE}/{service}/FeatureServer/0/query?'+urllib.parse.urlencode(q)
        d=get(url); batch=d.get('features',[])
        feats+=batch; print(name,'fetched',len(feats),flush=True)
        if len(batch)<2000: break
        off+=2000
        if off>200000: break
    os.makedirs(RAW,exist_ok=True)
    json.dump({'type':'FeatureCollection','features':feats},open(f'{RAW}/{name}.geojson','w'))
    return feats
def dem():
    os.makedirs(RAW,exist_ok=True); out=[]
    for tile in ['n47w123','n47w122']:
        path=f'{RAW}/{tile}.tif'
        if not os.path.exists(path):
            url=f'https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/13/TIFF/current/{tile}/USGS_13_{tile}.tif'
            print('dem',url,flush=True); urllib.request.urlretrieve(url,path)
        out.append(path)
    return out
def main():
    os.makedirs(ROOT,exist_ok=True)
    layers={k:fetch(k,v) for k,v in LAYERS.items()}
    dems=dem()
    # sample DEM
    ds=[rasterio.open(p) for p in dems]
    def elev(x,y):
        for d in ds:
            try:
                v=list(d.sample([(x,y)]))[0][0]
                if v>-1000: return float(v)
            except Exception: pass
        return 0.0
    # 2015 apex join by nearest centroid
    t15=[]
    for f in layers['buildings_2015']:
        if not f.get('geometry'): continue
        ap=f['properties'].get('BP99_APEX')
        if ap is None: continue
        g=shape(f['geometry']); t15.append((g.centroid.x,g.centroid.y,float(ap)))
    tree=STRtree([__import__('shapely').geometry.Point(x,y) for x,y,_ in t15]) if t15 else None
    buildings=[]; rejected=0; with_height=0
    for f in layers['buildings_2023']:
        if not f.get('geometry'): rejected+=1; continue
        try: g=shape(f['geometry'])
        except Exception: rejected+=1; continue
        if g.is_empty or g.area<8: rejected+=1; continue
        cx,cy=g.centroid.x,g.centroid.y
        apex=None
        if tree:
            i=tree.nearest(__import__('shapely').geometry.Point(cx,cy))
            x,y,ap=t15[int(i)]
            if ((cx-x)**2+(cy-y)**2)**0.5<25: apex=ap
        base=elev(cx,cy)
        if apex is None: h=12.0
        else: h=max(3.0,apex-base); with_height+=1
        ring=list(g.exterior.coords)[:-1]
        buildings.append({'id':len(buildings),'block':0,'centroid':[cx,cy],
          'perimeter':[[p[0],p[1]] for p in ring],'height_m':h,
          'floor_height_m':3.4,'category':'residential','year_built':0,
          'base_elevation_m':base})
    meta={'version':'1.1','city_id':'seattle','attribution':'City of Seattle GeoData (Building Outlines 2023/2015, SND, zoning, shoreline); USGS 3DEP','license':'Seattle open data / USGS public domain',
      'fields':{'epicenter':[1272000.0,234000.0],'ad_radius_m':2500,'ad_full_m':250,'glass_radius_m':900},
      'terrain':None,'water_bodies':None,'buildings':buildings}
    json.dump(meta,open(f'{ROOT}/seattle_v11.json','w'),separators=(',',':'))
    manifest={'sources':{k:f'{BASE}/{v}/FeatureServer/0' for k,v in LAYERS.items()},
      'dem':[f'USGS_13_{t}' for t in ['n47w123','n47w122']],
      'counts':{k:len(v) for k,v in layers.items()},
      'buildings':len(buildings),'rejected':rejected,'with_real_height':with_height,
      'height_coverage':with_height/max(1,len(buildings)),
      'region_envelope_2926':ENV,'notes':'2023 layer has no height; 2015 BP99_APEX joined within 25 m, base sampled from 3DEP'}
    json.dump(manifest,open(f'{ROOT}/manifest.json','w'),indent=2)
    print('SEATTLE',json.dumps(manifest['counts']),'buildings',len(buildings),'rejected',rejected,'height_cov',manifest['height_coverage'])
if __name__=='__main__': main()
