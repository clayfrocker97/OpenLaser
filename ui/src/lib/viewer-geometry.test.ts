import { describe, expect, it } from 'vitest';
import { axisAlignedBounds, boundsOfShapes, BoundsIndex, drawingPath, hitPath, inverseBounds, marqueeGroups, nearestShape, overlaps, smallShapeAround, transformedBounds } from './viewer-geometry';
import { boxOf, pathOf } from './svg';
import type { PreviewContour, Transform } from '../api';

describe('viewer bounds and cached paths', () => {
  it('selects a partially covered group member but excludes empty gaps and misses', () => {
    const contour = (x: number) => ({ paths: [{ kind: 'cut', points: [[x, 0], [x + 10, 10]] }] }) as PreviewContour;
    const shapes = new Map([[0, [contour(0), contour(100)]], [1, [contour(200)]]]);
    const index = new BoundsIndex(boundsOfShapes(shapes));
    expect(marqueeGroups(shapes, index, { minX: -2, minY: 2, maxX: 2, maxY: 4 })).toEqual([0]);
    expect(marqueeGroups(shapes, index, { minX: 50, minY: 2, maxX: 60, maxY: 4 })).toEqual([]);
    expect(marqueeGroups(shapes, index, { minX: -20, minY: 2, maxX: -10, maxY: 4 })).toEqual([]);
    expect(marqueeGroups(shapes, index, { minX: 109, minY: 2, maxX: 201, maxY: 4 })).toEqual([0, 1]);
  });

  it('keeps path output identical and includes a lead beyond the part bounds', () => {
    const body: [number, number][] = [[0, 0], [10, 0], [0, 10], [0, 0]];
    const entry: [number, number][] = [[-25, 0], [0, 0]];
    const contour = { paths: [{ kind: 'cut', points: body }, { kind: 'lead_in', points: entry }] } as PreviewContour;
    expect(drawingPath(body)).toBe(pathOf(body, false));
    expect(hitPath(contour)).toBe(`${pathOf(body, false)} ${pathOf(entry, false)}`);
    const bounds = boundsOfShapes(new Map([[8, [contour]]]));
    expect(bounds.get(8)).toEqual({ minX: -25, minY: 0, maxX: 10, maxY: 10 });
    expect(new BoundsIndex(bounds).query({ minX: -26, minY: -1, maxX: -24, maxY: 1 })).toEqual([8]);
  });

  it('uses exact bounds for scale/reflection and actual vertices for rotation', () => {
    const points: [number, number][] = [[0, 0], [10, 0], [0, 10]];
    const box = boxOf([points])!;
    const mirror: Transform = [-2, 0, 0, 2, 5, -7];
    expect(axisAlignedBounds(box, mirror)).toEqual(transformedBounds([points], mirror));
    const c = Math.SQRT1_2;
    const rotation: Transform = [c, c, -c, c, 0, 0];
    expect(axisAlignedBounds(box, rotation)).toBeNull();
    expect(transformedBounds([points], rotation)!.maxY).toBeCloseTo(10 * c);
    // Rotating the original bounding rectangle would incorrectly give 20*c.
  });

  it('matches exhaustive visibility, including boundaries, for a dense sheet', () => {
    const boxes = new Map(Array.from({ length: 2048 }, (_, g) => [g, { minX: g % 64 * 20, maxX: g % 64 * 20 + 17, minY: Math.floor(g / 64) * 20, maxY: Math.floor(g / 64) * 20 + 17 }]));
    const index = new BoundsIndex(boxes);
    for (let i = 0; i < 90; i++) {
      const camera = { minX: i * 15 - 100, minY: i * 7 - 30, maxX: i * 15 + 157, maxY: i * 7 + 197 };
      expect(index.query(camera).sort((a, b) => a - b)).toEqual([...boxes].filter(([, b]) => overlaps(b, camera)).map(([g]) => g));
    }
  });

  it('retains every visible transformed point when a selected sheet moves or rotates', () => {
    const camera = { minX: -10, minY: -12, maxX: 13, maxY: 19 };
    for (const angle of [0, .23, Math.PI / 2, 2.9]) for (const scale of [.1, 1, 12]) {
      const c = Math.cos(angle) * scale, s = Math.sin(angle) * scale;
      const m: Transform = [c, s, -s, c, 15, -3];
      const original = inverseBounds(camera, m)!;
      for (let x = -100; x < 100; x += 3) for (let y = -100; y < 100; y += 3) {
        const px = c * x - s * y + 15, py = s * x + c * y - 3;
        if (px >= camera.minX && px <= camera.maxX && py >= camera.minY && py <= camera.maxY) {
          expect(x >= original.minX - 1e-9 && x <= original.maxX + 1e-9 && y >= original.minY - 1e-9 && y <= original.maxY + 1e-9).toBe(true);
        }
      }
    }
    expect(inverseBounds(camera, [0, 0, 0, 0, 0, 0])).toBeNull();
  });

  it('takes the shape whose line is nearest a tap, not the one drawn on top', () => {
    const ring = (r: number, source: number) => ({ sources: [source], layer: 'CSK', paths: [{ kind: 'cut', points: Array.from({ length: 65 }, (_, i) => [r * Math.cos(i / 32 * Math.PI), r * Math.sin(i / 32 * Math.PI)]) }] }) as never;
    const shapes = new Map([[0, [ring(4, 1), ring(7, 2)]]]);
    const index = new BoundsIndex(boundsOfShapes(shapes));
    expect((nearestShape(shapes, index, [4.4, 0], 2)!.contour as { sources: number[] }).sources).toEqual([1]);
    expect((nearestShape(shapes, index, [6.2, 0], 2)!.contour as { sources: number[] }).sources).toEqual([2]);
    expect(nearestShape(shapes, index, [20, 20], 2)).toBeNull();
  });

  it('takes a small hole tapped inside, never the empty inside of a large outline', () => {
    const ring = (r: number, source: number) => ({ sources: [source], layer: 'CUT', paths: [{ kind: 'cut', points: Array.from({ length: 65 }, (_, i) => [r * Math.cos(i / 32 * Math.PI), r * Math.sin(i / 32 * Math.PI)]) }] }) as never;
    const shapes = new Map([[0, [ring(3, 1), ring(40, 2)]]]);
    const index = new BoundsIndex(boundsOfShapes(shapes));
    const around = (at: [number, number]) => (smallShapeAround(shapes, index, at, 20)?.contour as { sources: number[] } | undefined)?.sources;
    expect(around([0.5, 0.5])).toEqual([1]);
    expect(around([20, 0])).toBeUndefined();
    expect(around([60, 0])).toBeUndefined();
  });
});
