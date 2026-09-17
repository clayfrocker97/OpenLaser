"""Create reproducible full-bed DXFs in an isolated OpenLaser test library."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import urllib.parse
import urllib.request


def request(base, path, value=None, raw=None):
    body = raw if raw is not None else (json.dumps(value).encode() if value is not None else None)
    headers = {"Content-Type": "application/octet-stream" if raw is not None else "application/json"}
    req = urllib.request.Request(base + path, data=body, headers=headers)
    with urllib.request.urlopen(req, timeout=180) as response:
        return json.load(response)


def drawing(columns, rows, width, height):
    pairs = [(0, "SECTION"), (2, "HEADER"), (9, "$INSUNITS"), (70, 4), (0, "ENDSEC"),
             (0, "SECTION"), (2, "ENTITIES")]
    margin = 10
    cell_w, cell_h = (width - 2 * margin) / columns, (height - 2 * margin) / rows
    part_w, part_h = cell_w * .76, cell_h * .76
    if columns * rows == 1:
        part_w, part_h = 32, 22
    radius = min(part_w, part_h) * .15
    bulge = math.tan(math.pi / 8)
    for row in range(rows):
        for column in range(columns):
            x = margin + column * cell_w + (cell_w - part_w) / 2
            y = margin + row * cell_h + (cell_h - part_h) / 2
            vertices = [(radius, 0, 0), (part_w-radius, 0, bulge), (part_w, radius, 0),
                        (part_w, part_h-radius, bulge), (part_w-radius, part_h, 0),
                        (radius, part_h, bulge), (0, part_h-radius, 0), (0, radius, bulge)]
            pairs.extend([(0, "LWPOLYLINE"), (8, "plates"), (90, len(vertices)), (70, 1)])
            for px, py, curve in vertices:
                pairs.extend([(10, f"{x+px:.8f}"), (20, f"{y+py:.8f}"), (42, f"{curve:.10f}")])
            for fraction in [.32, .68]:
                pairs.extend([(0, "CIRCLE"), (8, "holes"), (10, f"{x+part_w*fraction:.8f}"),
                              (20, f"{y+part_h/2:.8f}"), (40, f"{min(part_w,part_h)*.09:.8f}")])
    pairs.extend([(0, "ENDSEC"), (0, "EOF")])
    return "".join(f"{code}\n{value}\n" for code, value in pairs).encode()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--backend", default="http://127.0.0.1:8111")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    assert urllib.parse.urlparse(args.backend).hostname == "127.0.0.1", "loopback test backend only"
    args.output.mkdir(parents=True, exist_ok=True)
    state = request(args.backend, "/api/state")
    extent = state["files"]["extent"]
    assert extent == [[0., 1371.], [0., 950.]], extent
    recipe = next(r for r in state["library"]["recipes"] if r["name"] == "Basswood" and r["laser"] == "co2")
    manifest = {"bed": extent, "recipe": recipe["id"], "features": state["draft"]["features"], "scenes": []}
    # Three contours per part: 1666 parts is just below the 5000-contour budget.
    for columns, rows in [(1, 1), (16, 16), (32, 32), (49, 34)]:
        count = columns * rows
        name = f"benchmark-{count}-rounded-plates.dxf"
        content = drawing(columns, rows, extent[0][1], extent[1][1])
        (args.output / name).write_bytes(content)
        imported = request(args.backend, "/api/parts?" + urllib.parse.urlencode({"name": name}), raw=content)
        item = {"id": imported["id"], "name": name, "parts": count, "contours": count * 3,
                "columns": columns, "rows": rows, "bytes": len(content), "sha256": hashlib.sha256(content).hexdigest()}
        manifest["scenes"].append(item)
        (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        print(json.dumps(item), flush=True)


if __name__ == "__main__":
    main()
