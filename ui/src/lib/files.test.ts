import { describe, expect, it } from 'vitest';
import { macMetadata } from './files';

describe('mac metadata', () => {
  it('skips the Finder copies a zip from a Mac carries', () => {
    expect(macMetadata('desk/__MACOSX/._Bin.dxf')).toBe(true);
    expect(macMetadata('desk/._Bin.dxf')).toBe(true);
    expect(macMetadata('._Bin.dxf')).toBe(true);
    expect(macMetadata('desk/Bin.dxf')).toBe(false);
    expect(macMetadata('desk/._old/Bin.dxf')).toBe(false);
  });
});
