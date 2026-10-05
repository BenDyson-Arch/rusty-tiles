"""Streaming OGR sources with explicit layer, axis and height semantics."""
import json
import math

import numpy as np
from osgeo import gdal, ogr, osr

gdal.UseExceptions()
ogr.UseExceptions()
osr.UseExceptions()


def geometry(g):
    if g is None or g.IsEmpty():
        raise ValueError('null/empty geometry is unsupported')
    if g.IsMeasured():
        raise ValueError('measured geometry is unsupported; retain or explicitly remove M')
    kind = ogr.GT_Flatten(g.GetGeometryType())
    width = 3 if ogr.GT_HasZ(g.GetGeometryType()) else 2
    if kind == ogr.wkbPoint:
        return dict(type='Point', coordinates=list(g.GetPoint(0)[:width]))
    if kind == ogr.wkbLineString:
        return dict(type='LineString', coordinates=[list(p[:width]) for p in g.GetPoints()])
    if kind == ogr.wkbPolygon:
        return dict(type='Polygon', coordinates=[[list(p[:width]) for p in r.GetPoints()] for r in g])
    multi = {ogr.wkbMultiPoint: 'MultiPoint', ogr.wkbMultiLineString: 'MultiLineString', ogr.wkbMultiPolygon: 'MultiPolygon'}
    if kind in multi:
        return dict(type=multi[kind], coordinates=[geometry(child)['coordinates'] for child in g])
    raise ValueError(f'unsupported geometry: {g.GetGeometryName()}')


def map_coordinates(g, fn):
    def walk(value):
        if value and isinstance(value[0], (int, float)):
            return fn(value)
        return [walk(child) for child in value]
    return dict(type=g['type'], coordinates=walk(g['coordinates']))


def coordinate_list(g):
    def walk(value):
        if value and isinstance(value[0], (int, float)):
            yield value
        else:
            for child in value:
                yield from walk(child)
    return list(walk(g['coordinates']))


def spatial_ref(text):
    srs = osr.SpatialReference()
    srs.SetFromUserInput(text)
    srs.SetAxisMappingStrategy(osr.OAMS_TRADITIONAL_GIS_ORDER)
    return srs


def transformation(source, target):
    options = osr.CoordinateTransformationOptions()
    options.SetBallparkAllowed(False)
    options.SetOnlyBest(True)
    return osr.CreateCoordinateTransformation(source, target, options)


class Reader:
    def __init__(self, args):
        self.args = args
        if getattr(args,'height_offset',None) is not None and not math.isfinite(args.height_offset):
            raise ValueError('height-offset must be finite')
        if getattr(args,'max_source_vertices',1000000)<1:
            raise ValueError('maxSourceVertices must be positive')
        gdal.SetConfigOption('PROJ_NETWORK', 'OFF')
        self.dataset = gdal.OpenEx(str(args.input), gdal.OF_VECTOR | gdal.OF_READONLY,
                                  open_options=['NATIVE_DATA=YES'] if str(args.input).lower().endswith(('.json','.geojson')) else [])
        if self.dataset is None:
            raise ValueError('OGR cannot open vector input')
        self.driver = self.dataset.GetDriver().ShortName
        spatial = [self.dataset.GetLayer(i) for i in range(self.dataset.GetLayerCount())
                   if ogr.GT_Flatten(self.dataset.GetLayer(i).GetGeomType()) != ogr.wkbNone]
        names = [layer.GetName() for layer in spatial]
        requested = getattr(args, 'layers', []) or []
        if requested and getattr(args, 'all_layers', False):
            raise ValueError('choose --layer or --all-layers, not both')
        if len(set(requested)) != len(requested):
            raise ValueError('duplicate layer selection')
        missing = set(requested)-set(names)
        if missing:
            raise ValueError(f'unknown spatial layer(s): {sorted(missing)}; available: {names}')
        if not requested and len(spatial)>1 and not getattr(args,'all_layers',False):
            raise ValueError(f'select --layer NAME (repeatable) or --all-layers; available: {names}')
        self.layers = [layer for layer in spatial if not requested or layer.GetName() in requested]
        if not self.layers:
            raise ValueError('input has no selected spatial layers')
        self.local = getattr(args,'source_crs',None) == 'local'
        self.frame = getattr(args,'_reuse_frame',None)
        self.anchor = getattr(args,'_reuse_anchor',None)
        self.on_feature_error = None
        self.schemas = {}
        self.layer_reports = []
        self.target = spatial_ref('EPSG:4978')
        self.geographic = spatial_ref('EPSG:4979')
        self.from_ecef = transformation(self.target, self.geographic)

    def register_type(self, name, kind):
        previous = self.schemas.get(name)
        if previous is not None and previous != kind:
            if {previous,kind} <= {'integer','real'}:
                kind = 'real'
            else:
                raise ValueError(f'incompatible scalar schemas for {name!r}; convert these layers separately')
        self.schemas[name] = kind

    def __iter__(self):
        for layer in self.layers:
            name = layer.GetName()
            source = (spatial_ref(self.args.source_crs) if getattr(self.args,'source_crs',None) and not self.local
                      else layer.GetSpatialRef())
            if not self.local and source is None:
                raise ValueError(f'layer {name!r} needs a declared CRS or --source-crs override')
            transform = None
            native_height = False
            if not self.local:
                source = source.Clone()
                source.SetAxisMappingStrategy(osr.OAMS_TRADITIONAL_GIS_ORDER)
                native_height = bool(source.IsCompound() or source.IsGeocentric() or source.GetAuthorityCode(None) == '4979')
                if not native_height:
                    source.PromoteTo3D()
                transform = transformation(source,self.target)
            if self.local and getattr(self.args,'height_offset',None) is not None:
                raise ValueError('local XYZ is in metres; height-offset is for geospatial placement')
            if native_height and getattr(self.args,'height_offset',None) is not None:
                raise ValueError('declared 3D/vertical CRS already defines heights; use a horizontal CRS override to apply height-offset')
            definition = layer.GetLayerDefn()
            fields = []
            for i in range(definition.GetFieldCount()):
                field = definition.GetFieldDefn(i)
                key, kind = field.GetName(), field.GetType()
                if key in ('_source_id','_source_layer'):
                    raise ValueError(f'reserved source property: {key}')
                scalar = {ogr.OFTInteger:'integer',ogr.OFTInteger64:'integer',ogr.OFTReal:'real',
                          ogr.OFTString:'string',ogr.OFTDate:'string',ogr.OFTTime:'string',ogr.OFTDateTime:'string'}.get(kind)
                if field.GetSubType() == ogr.OFSTBoolean:
                    scalar = 'boolean'
                if scalar is None:
                    raise ValueError(f'unsupported field type for {name}.{key}: {field.GetTypeName()}')
                fields.append((key,scalar))
                if self.driver != 'GeoJSON':
                    self.register_type(key,scalar)
            count = 0
            layer.ResetReading()
            for row in layer:
                fid = row.GetFID()
                prefix = f'layer {name!r}, FID {fid}: '
                source_id = fid
                try:
                    native = row.GetNativeData() if self.driver == 'GeoJSON' else None
                    if native:
                        source_id = json.loads(native).get('id',fid)
                    g = row.GetGeometryRef()
                    points = geometry(g)
                    source_points = coordinate_list(points)
                    if len(source_points)>getattr(self.args,'max_source_vertices',1000000):
                        raise ValueError('source feature exceeds maxSourceVertices; explicitly raise the limit or subdivide the source')
                    if not all(len(p) in (2,3) and all(math.isfinite(v) for v in p) for p in source_points):
                        raise ValueError('coordinates must be finite XYZ')
                    if not self.local and source.IsGeographic():
                        angular=source.GetAngularUnits()
                        if any(abs(p[0]*angular)>math.pi+1e-12 or abs(p[1]*angular)>math.pi/2+1e-12 for p in source_points):
                            raise ValueError('geographic coordinates outside longitude/latitude range')
                    has_z = bool(ogr.GT_HasZ(g.GetGeometryType()))
                    height = getattr(self.args,'height_offset',None)
                    if has_z and not self.local and not native_height and height is None and self.driver != 'GeoJSON':
                        raise ValueError('3D horizontal-CRS input requires explicit height-offset to ellipsoidal metres')
                    properties = {}
                    source_id = fid
                    native = row.GetNativeData() if self.driver == 'GeoJSON' else None
                    if native:
                        feature = json.loads(native)
                        properties = dict(feature.get('properties') or {})
                        source_id = feature.get('id',fid)
                        # Native properties preserve exact JSON integer IDs and scalar types.
                        for key,value in properties.items():
                            if key in ('_source_id','_source_layer'):
                                raise ValueError(f'reserved source property: {key}')
                            if value is None:
                                continue
                            kind = ('boolean' if type(value) is bool else 'integer' if type(value) is int
                                    else 'real' if type(value) is float else 'string' if type(value) is str else None)
                            if kind is None:
                                raise ValueError(f'unsupported complex property: {key}')
                            self.register_type(key,kind)
                    else:
                        for key,scalar in fields:
                            value = row.GetField(key)
                            if scalar == 'boolean' and value is not None:
                                value = bool(value)
                            properties[key] = value
                    xyz = np.array([list(p) if len(p)==3 else [*p,0.] for p in source_points],dtype=float)
                    if not self.local:
                        if not native_height:
                            xyz[:,2] += height or 0.
                        # OSR traditionally orders easting/northing or longitude/latitude;
                        # promoted horizontal CRS uses metre Z. Missing operations fail.
                        xyz = np.asarray(transform.TransformPoints(xyz.tolist()),dtype=float)[:,:3]
                    if not np.isfinite(xyz).all():
                        raise ValueError('coordinate operation produced nonfinite positions')
                    if self.anchor is None:
                        self.anchor = xyz[0].copy()
                        if self.local:
                            self.frame = np.eye(3)
                        else:
                            lon,lat,_ = self.from_ecef.TransformPoint(*self.anchor)
                            lon,lat = np.radians([lon,lat])
                            self.frame = np.array([[-np.sin(lon),np.cos(lon),0],
                                [-np.sin(lat)*np.cos(lon),-np.sin(lat)*np.sin(lon),np.cos(lat)],
                                [np.cos(lat)*np.cos(lon),np.cos(lat)*np.sin(lon),np.sin(lat)]])
                    xyz = ((xyz-self.anchor) @ self.frame.T)[:,[0,2,1]]*[1,1,-1]
                    iterator = iter(xyz.tolist())
                    points = map_coordinates(points,lambda p:next(iterator))
                    properties['_source_id'] = json.dumps(source_id,separators=(',',':'),allow_nan=False)
                    properties['_source_layer'] = name
                    count += 1
                    yield dict(properties=properties,geometry=points)
                except (ValueError,RuntimeError,TypeError) as error:
                    if self.on_feature_error is None:
                        raise ValueError(prefix+str(error)) from error
                    self.on_feature_error(dict(sourceLayer=name,sourceId=json.dumps(source_id,separators=(',',':')),reason=str(error)))
            self.layer_reports.append(dict(name=name,features=count,sourceCrs=None if self.local else source.ExportToWkt(),
                heightMode='local metres' if self.local else 'declared CRS' if native_height else 'explicit offset' if getattr(self.args,'height_offset',None) is not None else '2D ellipsoid zero' if self.driver != 'GeoJSON' else 'GeoJSON ellipsoidal metres',
                heightOffset=getattr(self.args,'height_offset',None)))
        self.schemas.update(_source_id='string',_source_layer='string')
