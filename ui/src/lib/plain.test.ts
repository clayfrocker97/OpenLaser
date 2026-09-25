import { describe, expect, it } from 'vitest';
import type { AlarmView } from '../api';
import { blockedControls, groupAlarms, plain } from './plain';

const alarm = (title: string, fix: string, moves_axes = false): AlarmView => ({
  active: true, relief: { label: moves_axes ? 'Home head' : 'Reset', moves_axes }, id: moves_axes ? 45 : null,
  source: 'head', label: title, title, fix, blocking: true, latched: false, age_seconds: 0,
});

describe('plain', () => {
  it('rewords developer text and keeps the original as a detail', () => {
    expect(plain('missing setting ManuParam.FC.ShortNoUpMaxLength')).toEqual({
      text: 'The machine files are missing a setting. Re-import the machine backup in Settings.',
      detail: 'missing setting ManuParam.FC.ShortNoUpMaxLength',
    });
    expect(plain('no reply to read of register 1000').text).toBe('The controller did not answer. Check its power and network cable.');
    expect(plain('the controller refused: write of register 0x65: the controller refused the request: Modbus exception 6 (controller busy)').text)
      .toBe('The controller refused a command, even when it was sent again. Check the machine before continuing.');
    expect(plain('controller group 1 bit 24 is set').text).toBe('A controller signal is set.');
  });

  it('leaves plain reasons as sentences without details', () => {
    expect(plain('home XY first')).toEqual({ text: 'Home the machine first.', detail: null });
    expect(plain('the sheet is too small')).toEqual({ text: 'The sheet is too small.', detail: null });
    expect(plain(null).text).toBe('');
  });

  it('names alarms, and names homing when homing clears them all', () => {
    expect(plain('alarms are active: Emergency stop pressed').text).toBe('Alarm: Emergency stop pressed.');
    expect(plain('alarms are active: Head fault; Head needs homing').text).toBe('2 alarms: Head fault and 1 more.');
    const head = [alarm('Head fault', 'Home the head. If it comes back…'), alarm('Head needs homing', 'Home the head.', true)];
    expect(plain('alarms are active: Head fault; Head needs homing', head).text).toBe('Head needs homing. Tap for details.');
  });
});

describe('blockedControls', () => {
  it('groups controls that share a reason', () => {
    const closed = { ok: false, reason: 'home XY first' };
    expect(blockedControls([['Start', closed], ['Frame', closed], ['Home', { ok: true, reason: null }]])).toEqual([
      { names: ['Start', 'Frame'], reason: { text: 'Home the machine first.', detail: null } },
    ]);
  });
});

describe('groupAlarms', () => {
  it('puts every row homing clears behind one Home head button', () => {
    const rows = [alarm('Head fault', 'Home the head.'), alarm('Head needs homing', 'Home the head.', true), alarm('Emergency stop pressed', 'Release it.')];
    const groups = groupAlarms(rows);
    expect(groups.map((group) => group.alarms.map((row) => row.title))).toEqual([['Head needs homing', 'Head fault'], ['Emergency stop pressed']]);
    expect(groups[0]!.relief.label).toBe('Home head');
  });

  it('keeps rows apart when nothing offers homing', () => {
    expect(groupAlarms([alarm('Head fault', 'Home the head.')])).toHaveLength(1);
  });
});
