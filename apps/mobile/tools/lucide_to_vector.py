#!/usr/bin/env python3
"""Convert Lucide SVG icons (24x24, stroke-based) into Android/Compose vector drawables.

Usage: lucide_to_vector.py <lucide icons dir> <out dir> name [name ...]
Supports the elements Lucide uses: path, line, polyline, polygon, rect, circle, ellipse.
"""
import sys, re, xml.etree.ElementTree as ET
from pathlib import Path

def f(v): return float(v or 0)

def to_path(el):
    tag = el.tag.split('}')[-1]
    a = el.attrib
    if tag == 'path':
        return a['d']
    if tag == 'line':
        return f"M{a['x1']} {a['y1']}L{a['x2']} {a['y2']}"
    if tag in ('polyline', 'polygon'):
        pts = re.split(r'[\s,]+', a['points'].strip())
        pairs = [f"{pts[i]} {pts[i+1]}" for i in range(0, len(pts) - 1, 2)]
        d = 'M' + 'L'.join(pairs)
        return d + ('Z' if tag == 'polygon' else '')
    if tag == 'circle':
        cx, cy, r = f(a['cx']), f(a['cy']), f(a['r'])
        return f"M{cx - r} {cy}a{r} {r} 0 1 0 {2*r} 0a{r} {r} 0 1 0 {-2*r} 0"
    if tag == 'ellipse':
        cx, cy, rx, ry = f(a['cx']), f(a['cy']), f(a['rx']), f(a['ry'])
        return f"M{cx - rx} {cy}a{rx} {ry} 0 1 0 {2*rx} 0a{rx} {ry} 0 1 0 {-2*rx} 0"
    if tag == 'rect':
        x, y, w, h = f(a.get('x')), f(a.get('y')), f(a['width']), f(a['height'])
        rx = f(a.get('rx', a.get('ry', 0))); ry = f(a.get('ry', a.get('rx', 0)))
        if rx == 0:
            return f"M{x} {y}h{w}v{h}h{-w}Z"
        return (f"M{x+rx} {y}h{w-2*rx}a{rx} {ry} 0 0 1 {rx} {ry}v{h-2*ry}a{rx} {ry} 0 0 1 {-rx} {ry}"
                f"h{-(w-2*rx)}a{rx} {ry} 0 0 1 {-rx} {-ry}v{-(h-2*ry)}a{rx} {ry} 0 0 1 {rx} {-ry}Z")
    return None

def convert(svg: Path) -> str:
    root = ET.parse(svg).getroot()
    paths = []
    for el in root.iter():
        d = to_path(el)
        if d:
            paths.append(d)
    body = "\n".join(
        f'    <path android:pathData="{d}" android:fillColor="#00000000" android:strokeColor="#FF000000"\n'
        f'        android:strokeWidth="2" android:strokeLineCap="round" android:strokeLineJoin="round" />'
        for d in paths)
    return ('<?xml version="1.0" encoding="utf-8"?>\n'
            f'<!-- Lucide "{svg.stem}" (ISC license), converted by tools/lucide_to_vector.py -->\n'
            '<vector xmlns:android="http://schemas.android.com/apk/res/android"\n'
            '    android:width="24dp" android:height="24dp" android:viewportWidth="24" android:viewportHeight="24">\n'
            f'{body}\n</vector>\n')

if __name__ == '__main__':
    src, out = Path(sys.argv[1]), Path(sys.argv[2])
    for name in sys.argv[3:]:
        (out / f"ic_{name.replace('-', '_')}.xml").write_text(convert(src / f"{name}.svg"))
