/** The same workflow and names appear on the desktop and phone. */
export const WORKFLOW = [
  { id: 'parts', number: '1', label: 'Parts' },
  { id: 'setup', number: '2', label: 'Setup' },
  { id: 'run', number: '3', label: 'Run' },
] as const;

export const SETTINGS = { id: 'machine', label: 'Settings' } as const;

/**
 * The settings sidebar, in order, with Advanced last. Stable page IDs keep
 * desktop and phone navigation in sync; `keywords` help the settings search
 * find a page by words that are not on it.
 */
export const SETTINGS_PAGES: ReadonlyArray<{ id: number; label: string; groups: readonly string[]; wide?: boolean; keywords?: string }> = [
  { id: 0, label: 'General', groups: ['display', 'checklists', 'interface'], keywords: 'units inches metric theme dark night hold time preflight postflight pause checklist layout' },
  { id: 1, label: 'Machine', groups: ['controller', 'laser', 'tests', 'axes', 'safety'], keywords: 'controller firmware laser fiber co2 head travel limits jog speed inputs door water test pulse gas valve pointer shutter mode switch' },
  { id: 2, label: 'Materials & processes', groups: ['materials', 'process', 'gas'], keywords: 'recipes material library process ini gas cost price nitrogen oxygen air flow' },
  { id: 3, label: 'Calibration', groups: ['head-calibration', 'matrix'], wide: true, keywords: 'matrix correction squareness head calibration' },
  { id: 4, label: 'Network & phones', groups: ['network', 'phones'], keywords: 'ip address adapter route udp lan wifi phone tablet remote' },
  { id: 5, label: 'Advanced controller parameters', groups: ['xml'], wide: true, keywords: 'xml backup parameters registers' },
];
