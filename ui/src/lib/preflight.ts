import { distance, unitLabel } from '../lib/units.svelte';
import type { Anchor, CheckAction } from '../api';

export const BED_POINTS: Array<[Anchor, string]> = [
  ['front_left', 'Front left'], ['front_center', 'Front center'], ['front_right', 'Front right'],
  ['left', 'Left center'], ['center', 'Bed center'], ['right', 'Right center'],
  ['back_left', 'Back left'], ['back_center', 'Back center'], ['back_right', 'Back right'],
];
export const ACTIONS: Array<[CheckAction['kind'] | 'none', string]> = [
  ['none', 'No action'], ['home', 'Home'], ['origin', 'Go to job origin'],
  ['move_to', 'Move to bed point'], ['move_xy', 'Move to XY'], ['calibrate', 'Calibrate head'], ['job_gas_test', 'Gas test'],
];
export const movesMachine = (action: CheckAction | null): boolean => !!action && ['home', 'origin', 'move_to', 'move_xy', 'calibrate'].includes(action.kind);
/** Every checklist action is held: it moves, opens gas, or sets the origin. */
export const holdKind = (action: CheckAction): 'move' | 'zero' => action.kind === 'set_origin' || action.kind === 'set_origin_at' ? 'zero' : 'move';
export const canAutoCheck = (action: CheckAction | null): boolean => !!action && !['gas_test', 'job_gas_test', 'set_origin', 'set_origin_at'].includes(action.kind);
export function actionName(action: CheckAction): string {
  const point = 'point' in action ? BED_POINTS.find(([p]) => p === action.point)?.[1].replace(/ \(.+\)/, '').toLowerCase() : '';
  if (action.kind === 'gas_test' || action.kind === 'job_gas_test') return 'Test gas';
  if (action.kind === 'move_xy') return `Move to ${distance(action.x)}, ${distance(action.y)} ${unitLabel('mm')}`;
  if (action.kind === 'move_to') return `Move to ${point}`;
  if (action.kind === 'set_origin' || action.kind === 'set_origin_at') return 'Verify sheet position on Run';
  return ACTIONS.find(([kind]) => kind === action.kind)?.[1] ?? '';
}
