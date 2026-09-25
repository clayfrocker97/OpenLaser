#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Writes fixtures/import: drawings authored for OpenLaser's import tests.

Each file imitates how a CAD program lays out its export -- AutoCAD R12 and
R2000+ sections, handles and subclass markers, Fusion 360 and SolidWorks
splines, ellipses and flipped extrusions, LibreCAD and QCAD entities,
Inkscape, Illustrator and Fusion SVG headers -- but every file is written
here, from the geometry below. No third-party file is copied.

The Rust tests under crates/openlaser-dxf/tests/cad_exports.rs and
crates/openlaser-svg/tests/cad_exports.rs hold the same numbers, so run this
only to change the fixtures on purpose:

    python3 scripts/import_fixtures.py
"""

import math
import os

OUT = os.path.join(os.path.dirname(__file__), "..", "fixtures", "import")


def num(value):
    """A number the way CAD programs write it: shortest round trip."""
    if isinstance(value, int):
        return str(value)
    return repr(float(value))


class Dxf:
    """Group code and value pairs, with AutoCAD's right-aligned codes."""

    def __init__(self, handles=True):
        self.lines = []
        self.handles = handles
        self.next = 0x100

    def pair(self, code, value):
        self.lines.append(f"{code:>3}")
        self.lines.append(num(value) if isinstance(value, (int, float)) else str(value))

    def pairs(self, *items):
        for code, value in items:
            self.pair(code, value)

    def handle(self):
        if self.handles:
            self.pair(5, f"{self.next:X}")
            self.next += 1

    def entity(self, kind, layer, subclass=None, extra=()):
        self.pair(0, kind)
        self.handle()
        if self.handles:
            self.pair(100, "AcDbEntity")
        self.pair(8, layer)
        for code, value in extra:
            self.pair(code, value)
        if self.handles and subclass:
            self.pair(100, subclass)

    def section(self, name):
        self.pairs((0, "SECTION"), (2, name))

    def endsec(self):
        self.pair(0, "ENDSEC")

    def text(self):
        return "\n".join(self.lines + ["  0", "EOF", ""])


def header(d, version, units=None, extra=()):
    d.section("HEADER")
    d.pairs((9, "$ACADVER"), (1, version))
    if units is not None:
        d.pairs((9, "$INSUNITS"), (70, units))
    for name, code, value in extra:
        d.pairs((9, name), (code, value))
    d.endsec()


def tables(d, layers):
    """A LAYER table: (name, colour, flags), colour negative when off."""
    d.section("TABLES")
    d.pairs((0, "TABLE"), (2, "LAYER"))
    d.handle()
    if d.handles:
        d.pair(100, "AcDbSymbolTable")
    d.pair(70, len(layers))
    for name, colour, flags in layers:
        d.pair(0, "LAYER")
        d.handle()
        if d.handles:
            d.pairs((100, "AcDbSymbolTableRecord"), (100, "AcDbLayerTableRecord"))
        d.pairs((2, name), (70, flags), (62, colour), (6, "CONTINUOUS"))
    d.pair(0, "ENDTAB")
    d.endsec()


def line(d, layer, a, b, extra=()):
    d.entity("LINE", layer, "AcDbLine", extra)
    d.pairs((10, a[0]), (20, a[1]), (30, 0.0), (11, b[0]), (21, b[1]), (31, 0.0))


def circle(d, layer, c, r, extrusion=None):
    d.entity("CIRCLE", layer, "AcDbCircle")
    d.pairs((10, c[0]), (20, c[1]), (30, 0.0), (40, r))
    if extrusion:
        d.pairs((210, 0.0), (220, 0.0), (230, extrusion))


def arc(d, layer, c, r, start, end, extrusion=None):
    d.entity("ARC", layer, "AcDbCircle")
    d.pairs((10, c[0]), (20, c[1]), (30, 0.0), (40, r))
    if extrusion:
        d.pairs((210, 0.0), (220, 0.0), (230, extrusion))
    if d.handles:
        d.pair(100, "AcDbArc")
    d.pairs((50, start), (51, end))


def lwpolyline(d, layer, points, closed=True, extrusion=None):
    """Points are (x, y) or (x, y, bulge)."""
    d.entity("LWPOLYLINE", layer, "AcDbPolyline")
    d.pairs((90, len(points)), (70, 1 if closed else 0), (43, 0.0))
    for p in points:
        d.pairs((10, p[0]), (20, p[1]))
        if len(p) > 2 and p[2]:
            d.pair(42, p[2])
    if extrusion:
        d.pairs((210, 0.0), (220, 0.0), (230, extrusion))


def polyline_r12(d, layer, points, closed=True, flags=0, vertex_flags=None):
    d.entity("POLYLINE", layer)
    d.pairs((66, 1), (10, 0.0), (20, 0.0), (30, 0.0), (70, (1 if closed else 0) | flags))
    for i, p in enumerate(points):
        d.entity("VERTEX", layer)
        d.pairs((10, p[0]), (20, p[1]), (30, 0.0))
        if len(p) > 2 and p[2]:
            d.pair(42, p[2])
        if vertex_flags:
            d.pair(70, vertex_flags[i])
    d.entity("SEQEND", layer)


def spline(d, layer, degree, knots, controls, weights=None, fit=(), flags=8):
    d.entity("SPLINE", layer, "AcDbSpline")
    d.pairs((210, 0.0), (220, 0.0), (230, 1.0), (70, flags | (4 if weights else 0)))
    d.pairs((71, degree), (72, len(knots)), (73, len(controls)), (74, len(fit)))
    d.pairs((42, 1e-10), (43, 1e-10), (44, 1e-10))
    for k in knots:
        d.pair(40, k)
    if weights:
        for w in weights:
            d.pair(41, w)
    for p in controls:
        d.pairs((10, p[0]), (20, p[1]), (30, 0.0))
    for p in fit:
        d.pairs((11, p[0]), (21, p[1]), (31, 0.0))


def ellipse(d, layer, c, major, ratio, start=0.0, end=2 * math.pi, extrusion=1.0):
    d.entity("ELLIPSE", layer, "AcDbEllipse")
    d.pairs((10, c[0]), (20, c[1]), (30, 0.0), (11, major[0]), (21, major[1]), (31, 0.0))
    d.pairs((210, 0.0), (220, 0.0), (230, extrusion), (40, ratio), (41, start), (42, end))


def text(d, layer, at, height, value):
    d.entity("TEXT", layer, "AcDbText")
    d.pairs((10, at[0]), (20, at[1]), (30, 0.0), (40, height), (1, value))
    if d.handles:
        d.pair(100, "AcDbText")


def dimension(d, layer, a, b):
    d.entity("DIMENSION", layer, "AcDbDimension")
    d.pairs((2, "*D1"), (10, b[0]), (20, b[1] + 10), (30, 0.0), (70, 32), (1, ""))
    d.pairs((13, a[0]), (23, a[1]), (33, 0.0), (14, b[0]), (24, b[1]), (34, 0.0))


def insert(d, layer, block, at, scale=(1.0, 1.0), rotation=0.0, array=None, extrusion=None):
    d.entity("INSERT", layer, "AcDbBlockReference")
    d.pairs((2, block), (10, at[0]), (20, at[1]), (30, 0.0))
    if scale != (1.0, 1.0):
        d.pairs((41, scale[0]), (42, scale[1]), (43, 1.0))
    if rotation:
        d.pair(50, rotation)
    if array:
        columns, rows, dx, dy = array
        d.pairs((70, columns), (71, rows), (44, dx), (45, dy))
    if extrusion:
        d.pairs((210, 0.0), (220, 0.0), (230, extrusion))


def begin_block(d, name, base=(0.0, 0.0), flags=0):
    d.pair(0, "BLOCK")
    d.handle()
    if d.handles:
        d.pair(100, "AcDbEntity")
    d.pair(8, "0")
    if d.handles:
        d.pair(100, "AcDbBlockBegin")
    d.pairs((2, name), (70, flags), (10, base[0]), (20, base[1]), (30, 0.0), (3, name), (1, ""))


def end_block(d):
    d.pair(0, "ENDBLK")
    d.handle()
    if d.handles:
        d.pairs((100, "AcDbEntity"), (8, "0"), (100, "AcDbBlockEnd"))


def space_blocks(d):
    """R2000+ files carry the model and paper space block definitions."""
    for name in ("*Model_Space", "*Paper_Space"):
        begin_block(d, name)
        end_block(d)


def rectangle(x, y, w, h):
    return [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]


def write(name, content):
    with open(os.path.join(OUT, name), "w", newline="\n") as f:
        f.write(content)


# DXF ---------------------------------------------------------------------------


def autocad_bracket():
    """AutoCAD 2000 mm bracket: a bulged outline, holes, a dimension."""
    d = Dxf()
    header(d, "AC1015", 4, [("$MEASUREMENT", 70, 1)])
    tables(d, [("0", 7, 0), ("CUT", 1, 0), ("DIM", 3, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    lwpolyline(d, "CUT", [(0.0, 0.0), (80.0, 0.0), (80.0, 30.0, 0.41421356237309503), (70.0, 40.0), (0.0, 40.0)])
    circle(d, "CUT", (15.0, 20.0), 4.25)
    circle(d, "CUT", (60.0, 20.0), 4.25)
    dimension(d, "DIM", (0.0, 0.0), (80.0, 0.0))
    d.endsec()
    write("autocad-2000-bracket-mm.dxf", d.text())


def autocad_r12_flange():
    """AutoCAD R12: no $INSUNITS, drawn in inches, heavy polylines."""
    d = Dxf(handles=False)
    header(d, "AC1009", None, [("$EXTMIN", 10, 0.0), ("$EXTMAX", 10, 2.5)])
    tables(d, [("0", 7, 0), ("OUTLINE", 7, 0)])
    d.section("ENTITIES")
    # A 2.5 in square flange with rounded corners (bulge of a quarter turn).
    b = math.tan(math.pi / 8)
    r = 0.25
    polyline_r12(d, "OUTLINE", [
        (r, 0.0), (2.5 - r, 0.0, b), (2.5, r), (2.5, 2.5 - r, b), (2.5 - r, 2.5),
        (r, 2.5, b), (0.0, 2.5 - r), (0.0, r, b),
    ])
    circle(d, "OUTLINE", (1.25, 1.25), 0.5)
    for x, y in [(0.35, 0.35), (2.15, 0.35), (2.15, 2.15), (0.35, 2.15)]:
        circle(d, "OUTLINE", (x, y), 0.125)
    d.endsec()
    write("autocad-r12-flange-inch.dxf", d.text())


def tab_block(d):
    """A tab: a 20 × 10 plate with a slot arc, base point on its left edge."""
    begin_block(d, "TAB", (0.0, 5.0))
    lwpolyline(d, "0", rectangle(0.0, 0.0, 20.0, 10.0))
    arc(d, "HOLES", (14.0, 5.0), 2.0, 270.0, 90.0)
    line(d, "HOLES", (14.0, 3.0), (10.0, 3.0))
    line(d, "HOLES", (10.0, 7.0), (14.0, 7.0))
    arc(d, "HOLES", (10.0, 5.0), 2.0, 90.0, 270.0)
    end_block(d)


def autocad_blocks():
    """AutoCAD 2018: one block placed plain, rotated, scaled and mirrored."""
    d = Dxf()
    header(d, "AC1032", 4)
    tables(d, [("0", 7, 0), ("PARTS", 1, 0), ("HOLES", 2, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    tab_block(d)
    d.endsec()
    d.section("ENTITIES")
    insert(d, "PARTS", "TAB", (0.0, 0.0))
    insert(d, "PARTS", "TAB", (40.0, 0.0), rotation=90.0)
    insert(d, "PARTS", "TAB", (60.0, 0.0), scale=(1.5, 1.5))
    insert(d, "PARTS", "TAB", (120.0, 0.0), scale=(-1.0, 1.0))
    insert(d, "PARTS", "TAB", (130.0, 30.0), scale=(1.0, -1.0), rotation=180.0)
    d.endsec()
    write("autocad-2018-blocks-mirrored.dxf", d.text())


def autocad_minsert():
    """An arrayed block reference, 4 columns by 3 rows, rotated 30°."""
    d = Dxf()
    header(d, "AC1027", 4)
    tables(d, [("0", 7, 0), ("PARTS", 1, 0), ("HOLES", 2, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    tab_block(d)
    d.endsec()
    d.section("ENTITIES")
    insert(d, "PARTS", "TAB", (0.0, 0.0), rotation=30.0, array=(4, 3, 25.0, 15.0))
    d.endsec()
    write("autocad-2013-minsert-array.dxf", d.text())


def autocad_nested():
    """Nested blocks: a PAIR of tabs placed twice; attributes follow one."""
    d = Dxf()
    header(d, "AC1018", 4)
    tables(d, [("0", 7, 0), ("PARTS", 1, 0), ("HOLES", 2, 0), ("LABELS", 4, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    tab_block(d)
    begin_block(d, "PAIR", (0.0, 0.0))
    insert(d, "0", "TAB", (0.0, 5.0))
    insert(d, "0", "TAB", (0.0, 25.0), scale=(1.0, -1.0))
    end_block(d)
    d.endsec()
    d.section("ENTITIES")
    insert(d, "PARTS", "PAIR", (0.0, 0.0))
    d.entity("INSERT", "PARTS", "AcDbBlockReference", [(66, 1)])
    d.pairs((2, "PAIR"), (10, 50.0), (20, 0.0), (30, 0.0), (50, 90.0))
    d.entity("ATTRIB", "LABELS", "AcDbText")
    d.pairs((10, 50.0), (20, 0.0), (30, 0.0), (40, 2.5), (1, "P-104"), (2, "PARTNO"), (70, 0))
    d.entity("SEQEND", "PARTS")
    d.endsec()
    write("autocad-2004-nested-blocks.dxf", d.text())


def autocad_hidden_layers():
    """Layers turned off and frozen hold notes and construction lines."""
    d = Dxf()
    header(d, "AC1015", 4)
    tables(d, [("0", 7, 0), ("CUT", 7, 0), ("CONSTRUCTION", -8, 0), ("NOTES", 3, 1), ("ETCH", 5, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    lwpolyline(d, "CUT", rectangle(0.0, 0.0, 100.0, 60.0))
    circle(d, "CUT", (50.0, 30.0), 12.0)
    line(d, "CONSTRUCTION", (-10.0, 30.0), (110.0, 30.0))
    line(d, "CONSTRUCTION", (50.0, -10.0), (50.0, 70.0))
    text(d, "NOTES", (0.0, 70.0), 5.0, "PLATE 3MM S235")
    lwpolyline(d, "ETCH", rectangle(5.0, 5.0, 10.0, 5.0))
    line(d, "CUT", (0.0, 0.0), (100.0, 60.0), extra=[(60, 1)])
    d.endsec()
    write("autocad-2000-hidden-layers.dxf", d.text())


def autocad_paper_space():
    """A layout's title block sits in paper space beside the model."""
    d = Dxf()
    header(d, "AC1021", 4)
    tables(d, [("0", 7, 0), ("CUT", 7, 0), ("TITLE", 7, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    lwpolyline(d, "CUT", rectangle(0.0, 0.0, 50.0, 50.0))
    for a, b in [((0.0, 0.0), (420.0, 0.0)), ((420.0, 0.0), (420.0, 297.0)), ((420.0, 297.0), (0.0, 297.0)), ((0.0, 297.0), (0.0, 0.0))]:
        line(d, "TITLE", a, b, extra=[(67, 1)])
    d.endsec()
    write("autocad-2007-paper-space.dxf", d.text())


# The splines' numbers are shared with the accuracy tests.
FUSION_BEZIER = [(0.0, 0.0), (20.0, 60.0), (80.0, -40.0), (100.0, 20.0)]
FUSION_CONTROLS = [(0.0, 0.0), (10.0, 25.0), (30.0, 30.0), (50.0, 0.0), (70.0, -30.0), (90.0, -25.0), (100.0, 0.0)]
FUSION_KNOTS = [0.0, 0.0, 0.0, 0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0]


def fusion_splines():
    """Fusion 360 sketch export: clamped cubic splines and a closed profile."""
    d = Dxf()
    header(d, "AC1027", 4)
    tables(d, [("0", 7, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    spline(d, "0", 3, [0.0] * 4 + [1.0] * 4, FUSION_BEZIER)
    spline(d, "0", 3, FUSION_KNOTS, [(x, y + 100.0) for x, y in FUSION_CONTROLS])
    # A closed wave profile: a spline along the top, lines back.
    spline(d, "0", 3, FUSION_KNOTS, [(x, y + 200.0) for x, y in FUSION_CONTROLS])
    line(d, "0", (100.0, 200.0), (100.0, 160.0))
    line(d, "0", (100.0, 160.0), (0.0, 160.0))
    line(d, "0", (0.0, 160.0), (0.0, 200.0))
    d.endsec()
    write("fusion360-spline-sketch.dxf", d.text())


def fusion_ellipses():
    """Fusion 360: full ellipses, an elliptical slot and a tilted ellipse."""
    d = Dxf()
    header(d, "AC1027", 4)
    tables(d, [("0", 7, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    ellipse(d, "0", (50.0, 30.0), (40.0, 0.0), 0.5)
    ellipse(d, "0", (150.0, 30.0), (20.0, 20.0), 0.3)
    # A slot: two half ellipses joined by lines.
    ellipse(d, "0", (20.0, 100.0), (10.0, 0.0), 0.6, math.pi / 2, 3 * math.pi / 2)
    ellipse(d, "0", (80.0, 100.0), (10.0, 0.0), 0.6, 3 * math.pi / 2, math.pi / 2)
    line(d, "0", (20.0, 94.0), (80.0, 94.0))
    line(d, "0", (80.0, 106.0), (20.0, 106.0))
    d.endsec()
    write("fusion360-ellipses.dxf", d.text())


def solidworks_flat_pattern():
    """SolidWorks flat pattern in inches: lines and arcs that miss by a few
    thousandths of a millimetre and a bend line drawn twice."""
    d = Dxf()
    header(d, "AC1015", 1)
    tables(d, [("0", 7, 0), ("BEND", 2, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    gap = 0.0008  # inches: about 0.02 mm
    line(d, "0", (0.0, 0.0), (4.0, 0.0))
    line(d, "0", (4.0 + gap, 0.0), (4.0, 1.75))
    arc(d, "0", (3.75, 1.75), 0.25, 0.0, 90.0)
    line(d, "0", (3.75, 2.0), (0.25, 2.0 + gap))
    arc(d, "0", (0.25, 1.75), 0.25, 90.0, 180.0)
    line(d, "0", (0.0, 1.75), (0.0, 0.0))
    circle(d, "0", (2.0, 1.0), 0.375)
    line(d, "BEND", (0.0, 1.0), (4.0, 1.0))
    line(d, "BEND", (4.0, 1.0), (0.0, 1.0))
    line(d, "BEND", (1.0, 1.0), (3.0, 1.0))
    d.endsec()
    write("solidworks-flat-pattern-inch.dxf", d.text())


def solidworks_flipped():
    """SolidWorks drawing view with arcs and circles on extrusion (0,0,-1)."""
    d = Dxf()
    header(d, "AC1015", 4)
    tables(d, [("0", 7, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    # In object coordinates the X axis points along world -X: a tab whose
    # rounded end, drawn flipped, reaches x = 50 in the world.
    line(d, "0", (0.0, 0.0), (40.0, 0.0))
    arc(d, "0", (-40.0, 10.0), 10.0, 90.0, 270.0, extrusion=-1.0)
    line(d, "0", (40.0, 20.0), (0.0, 20.0))
    line(d, "0", (0.0, 20.0), (0.0, 0.0))
    circle(d, "0", (-20.0, 10.0), 5.0, extrusion=-1.0)
    lwpolyline(d, "0", rectangle(-10.0, 30.0, -20.0, 10.0), extrusion=-1.0)
    d.endsec()
    write("solidworks-flipped-extrusion.dxf", d.text())


def librecad_gasket():
    """LibreCAD 2: LWPOLYLINE gasket with bulged ends and bolt holes."""
    d = Dxf()
    header(d, "AC1015", 4, [("$DWGCODEPAGE", 3, "ANSI_1252")])
    tables(d, [("0", 7, 0), ("gasket", 7, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    lwpolyline(d, "gasket", [(0.0, 0.0), (60.0, 0.0, 1.0), (60.0, 30.0), (0.0, 30.0, 1.0)])
    lwpolyline(d, "gasket", [(15.0, 10.0), (45.0, 10.0, 1.0), (45.0, 20.0), (15.0, 20.0, 1.0)])
    circle(d, "gasket", (-7.5, 15.0), 3.0)
    circle(d, "gasket", (67.5, 15.0), 3.0)
    d.endsec()
    write("librecad-gasket.dxf", d.text())


QCAD_FIT = [(0.0, 0.0), (25.0, 15.0), (50.0, 0.0), (75.0, -15.0), (100.0, 0.0)]


def qcad_fit_points():
    """QCAD: a spline given only by fit points, and a closed one."""
    d = Dxf()
    header(d, "AC1015", 4)
    tables(d, [("0", 7, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    spline(d, "0", 3, [], [], fit=QCAD_FIT)
    loop = [(x, y + 60.0) for x, y in [(0.0, 0.0), (30.0, -10.0), (60.0, 0.0), (60.0, 30.0), (30.0, 40.0), (0.0, 30.0)]]
    spline(d, "0", 3, [], [], fit=loop, flags=8 | 1)
    d.endsec()
    write("qcad-fit-point-splines.dxf", d.text())


def qcad_duplicates():
    """QCAD: a plate whose outline was pasted twice and a line overlapping."""
    d = Dxf()
    header(d, "AC1015", 4)
    tables(d, [("0", 7, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    outline = rectangle(0.0, 0.0, 80.0, 50.0)
    lwpolyline(d, "0", outline)
    lwpolyline(d, "0", outline[2:] + outline[:2])
    circle(d, "0", (40.0, 25.0), 10.0)
    circle(d, "0", (40.0, 25.0), 10.0)
    arc(d, "0", (40.0, 25.0), 10.0, 0.0, 180.0)
    line(d, "0", (100.0, 0.0), (150.0, 0.0))
    line(d, "0", (110.0, 0.0), (130.0, 0.0))
    d.endsec()
    write("qcad-duplicates.dxf", d.text())


def r12_spline_polyline():
    """R12 spline-fit polyline: frame points and the vertices fitted along it."""
    d = Dxf(handles=False)
    header(d, "AC1009", 4)
    tables(d, [("0", 7, 0)])
    d.section("ENTITIES")
    frame = [(0.0, 0.0), (20.0, 40.0), (60.0, 40.0), (80.0, 0.0)]
    # The cubic's points at 16 steps, as AutoCAD's SPLINESEGS writes them.
    fitted = []
    for k in range(17):
        t = k / 16
        u = 1 - t
        fitted.append((
            frame[0][0] * u ** 3 + 3 * frame[1][0] * u * u * t + 3 * frame[2][0] * u * t * t + frame[3][0] * t ** 3,
            frame[0][1] * u ** 3 + 3 * frame[1][1] * u * u * t + 3 * frame[2][1] * u * t * t + frame[3][1] * t ** 3,
        ))
    points = frame + fitted
    polyline_r12(d, "0", points, closed=False, flags=4, vertex_flags=[16] * 4 + [8] * 17)
    d.endsec()
    write("autocad-r12-spline-fit-polyline.dxf", d.text())


def rational_circle():
    """A NURBS circle of radius 25: degree 2, nine controls, weights √½."""
    d = Dxf()
    header(d, "AC1024", 4)
    tables(d, [("0", 7, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    c, r = (40.0, 40.0), 25.0
    corners = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1), (1, 0)]
    controls = [(c[0] + r * x, c[1] + r * y) for x, y in corners]
    w = math.sqrt(0.5)
    weights = [1.0, w, 1.0, w, 1.0, w, 1.0, w, 1.0]
    knots = [0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0]
    spline(d, "0", 2, knots, controls, weights, flags=8 | 1)
    d.endsec()
    write("rhino-rational-circle.dxf", d.text())


def stretched_block():
    """A block of holes inserted with unequal scale: circles become ellipses."""
    d = Dxf()
    header(d, "AC1015", 4)
    tables(d, [("0", 7, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    begin_block(d, "HOLE")
    circle(d, "0", (0.0, 0.0), 10.0)
    arc(d, "0", (30.0, 0.0), 10.0, 0.0, 180.0)
    end_block(d)
    d.endsec()
    d.section("ENTITIES")
    insert(d, "0", "HOLE", (0.0, 0.0), scale=(2.0, 1.0))
    insert(d, "0", "HOLE", (0.0, 50.0), scale=(1.0, 0.5), rotation=45.0)
    d.endsec()
    write("autocad-2000-stretched-block.dxf", d.text())


def open_and_crossing():
    """A profile left open by half a millimetre and a bow tie that crosses."""
    d = Dxf()
    header(d, "AC1015", 4)
    tables(d, [("0", 7, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    lwpolyline(d, "0", [(0.0, 0.0), (40.0, 0.0), (40.0, 30.0), (0.0, 30.0), (0.0, 0.5)], closed=False)
    lwpolyline(d, "0", [(60.0, 0.0), (100.0, 30.0), (100.0, 0.0), (60.0, 30.0)])
    d.endsec()
    write("autocad-2000-open-and-crossing.dxf", d.text())


def oversize():
    """A 4 m × 2 m sheet outline: larger than most beds."""
    d = Dxf()
    header(d, "AC1015", 4)
    tables(d, [("0", 7, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    d.endsec()
    d.section("ENTITIES")
    lwpolyline(d, "0", rectangle(0.0, 0.0, 4000.0, 2000.0))
    circle(d, "0", (2000.0, 1000.0), 250.0)
    d.endsec()
    write("autocad-2000-oversize-sheet.dxf", d.text())


def mixed_sources():
    """DraftSight-style drawing mixing every entity kind on several layers."""
    d = Dxf()
    header(d, "AC1015", 4)
    tables(d, [("0", 7, 0), ("OUTER", 1, 0), ("INNER", 2, 0), ("ENGRAVE", 3, 0)])
    d.section("BLOCKS")
    space_blocks(d)
    begin_block(d, "SLOT", (0.0, 0.0))
    lwpolyline(d, "0", [(0.0, 0.0), (10.0, 0.0, 1.0), (10.0, 4.0), (0.0, 4.0, 1.0)])
    end_block(d)
    d.endsec()
    d.section("ENTITIES")
    lwpolyline(d, "OUTER", [(0.0, 0.0), (120.0, 0.0), (120.0, 80.0), (0.0, 80.0)])
    ellipse(d, "INNER", (30.0, 40.0), (15.0, 0.0), 0.5)
    spline(d, "ENGRAVE", 3, FUSION_KNOTS, [(10.0 + x * 0.5, 65.0 + y * 0.2) for x, y in FUSION_CONTROLS])
    for k in range(3):
        insert(d, "INNER", "SLOT", (70.0, 15.0 + 20.0 * k))
    d.endsec()
    write("draftsight-mixed-layers.dxf", d.text())


# SVG ---------------------------------------------------------------------------

INKSCAPE_HEAD = (
    '<?xml version="1.0" encoding="UTF-8" standalone="no"?>\n'
    '<!-- Created with Inkscape (http://www.inkscape.org/) -->\n'
)
INKSCAPE_NS = (
    'xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" '
    'xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" '
    'xmlns="http://www.w3.org/2000/svg" xmlns:svg="http://www.w3.org/2000/svg"'
)


def inkscape_mm():
    write("inkscape-1.3-plate-mm.svg", INKSCAPE_HEAD + f"""<svg width="120mm" height="80mm" viewBox="0 0 120 80" version="1.1" id="svg1" inkscape:version="1.3 (0e150ed6c4, 2023-07-21)" sodipodi:docname="plate.svg" {INKSCAPE_NS}>
  <sodipodi:namedview id="namedview1" pagecolor="#ffffff" inkscape:document-units="mm" />
  <defs id="defs1" />
  <g inkscape:label="Layer 1" inkscape:groupmode="layer" id="layer1">
    <rect style="fill:none;stroke:#000000;stroke-width:0.2" id="rect1" width="110" height="70" x="5" y="5" ry="6" />
    <circle style="fill:none;stroke:#000000;stroke-width:0.2" id="path1" cx="30" cy="40" r="12" />
    <ellipse style="fill:none;stroke:#000000;stroke-width:0.2" id="path2" cx="80" cy="40" rx="20" ry="8" />
    <path style="fill:none;stroke:#000000;stroke-width:0.2" d="m 60,15 a 10,10 0 0 1 10,10 10,10 0 0 1 -10,10" id="path3" />
  </g>
</svg>
""")


def inkscape_px():
    write("inkscape-0.92-bracket-px.svg", INKSCAPE_HEAD + f"""<svg width="378" height="189" viewBox="0 0 378 189" version="1.1" id="svg8" inkscape:version="0.92.4 (5da689c313, 2019-01-14)" {INKSCAPE_NS}>
  <g inkscape:label="Layer 1" inkscape:groupmode="layer" id="layer1">
    <path style="fill:none;stroke:#000000" d="M 0,0 H 378 V 189 H 0 Z" id="outline" />
    <circle style="fill:none;stroke:#000000" cx="94.5" cy="94.5" r="37.8" id="hole" />
  </g>
</svg>
""")


def illustrator():
    write("illustrator-cc-logo-72dpi.svg", """<?xml version="1.0" encoding="utf-8"?>
<!-- Generator: Adobe Illustrator 27.5.0, SVG Export Plug-In . SVG Version: 6.00 Build 0)  -->
<svg version="1.1" id="Layer_1" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" x="0px" y="0px"
\t viewBox="0 0 288 144" style="enable-background:new 0 0 288 144;" xml:space="preserve" width="288px" height="144px">
<style type="text/css">
\t.st0{fill:none;stroke:#000000;stroke-miterlimit:10;}
</style>
<rect x="0" y="0" class="st0" width="288" height="144"/>
<circle class="st0" cx="72" cy="72" r="36"/>
<ellipse class="st0" cx="198" cy="72" rx="54" ry="27"/>
<path class="st0" d="M144,18c14.9,0,27,12.1,27,27s-12.1,27-27,27"/>
</svg>
""")


def unknown_px():
    write("generic-export-unitless.svg", """<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200" viewBox="0 0 400 200">
  <g id="cut" fill="none" stroke="#ff0000" stroke-width="0.5">
    <rect x="10" y="10" width="380" height="180" rx="20"/>
    <circle cx="100" cy="100" r="40"/>
    <circle cx="300" cy="100" r="40"/>
  </g>
</svg>
""")


def fusion_svg():
    write("fusion360-sketch-cm.svg", """<?xml version="1.0" encoding="utf-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="10cm" height="6cm" viewBox="0 0 10 6">
<g id="Sketch1" fill="none" stroke="black" stroke-width="0.01">
<path d="M0,0 L10,0 L10,6 L0,6 Z"/>
<path d="M2,3 A1,1 0 0,1 4,3 A1,1 0 0,1 2,3 Z"/>
<path d="M6,1 C7,0 9,2 8,3 C7,4 9,5 7,5"/>
</g>
</svg>
""")


def transforms():
    write("inkscape-1.2-transforms.svg", INKSCAPE_HEAD + f"""<svg width="100mm" height="100mm" viewBox="0 0 100 100" version="1.1" inkscape:version="1.2.2" {INKSCAPE_NS}>
  <g inkscape:groupmode="layer" id="layer1" transform="translate(10,10)">
    <g transform="rotate(30 20 20)"><rect x="0" y="0" width="40" height="20" rx="4" fill="none" stroke="black"/></g>
    <g transform="scale(2,1)"><circle cx="15" cy="60" r="10" fill="none" stroke="black"/></g>
    <g transform="matrix(-1,0,0,1,90,0)"><path d="M0 30 A15 15 0 0 0 30 30" fill="none" stroke="black"/></g>
  </g>
</svg>
""")


def svg_repairs():
    write("illustrator-cs6-gaps-duplicates.svg", """<?xml version="1.0" encoding="utf-8"?>
<!-- Generator: Adobe Illustrator 16.0.0, SVG Export Plug-In . SVG Version: 6.00 Build 0)  -->
<svg version="1.1" xmlns="http://www.w3.org/2000/svg" width="200pt" height="100pt" viewBox="0 0 200 100">
<g id="Cut">
\t<polyline fill="none" stroke="#000000" points="10,10 190,10 190,90 "/>
\t<polyline fill="none" stroke="#000000" points="190.03,90 10,90 10,10.02 "/>
\t<line fill="none" stroke="#000000" x1="10" y1="10" x2="190" y2="10"/>
\t<circle fill="none" stroke="#000000" cx="100" cy="50" r="20"/>
\t<circle fill="none" stroke="#000000" cx="100" cy="50" r="20"/>
</g>
</svg>
""")


def main():
    os.makedirs(OUT, exist_ok=True)
    for build in [
        autocad_bracket, autocad_r12_flange, autocad_blocks, autocad_minsert, autocad_nested,
        autocad_hidden_layers, autocad_paper_space, fusion_splines, fusion_ellipses,
        solidworks_flat_pattern, solidworks_flipped, librecad_gasket, qcad_fit_points,
        qcad_duplicates, r12_spline_polyline, rational_circle, stretched_block,
        open_and_crossing, oversize, mixed_sources, inkscape_mm, inkscape_px, illustrator,
        unknown_px, fusion_svg, transforms, svg_repairs,
    ]:
        build()


if __name__ == "__main__":
    main()
