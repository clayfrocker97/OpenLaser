/** The same workflow and names appear on the desktop and phone. */
export const WORKFLOW = [
  { id: 'parts', number: '1', label: 'Parts' },
  { id: 'setup', number: '2', label: 'Setup' },
  { id: 'run', number: '3', label: 'Run' },
] as const;

export const SETTINGS = { id: 'machine', label: 'Settings' } as const;

/** Stable page IDs also keep desktop and phone settings navigation in sync. */
export const SETTINGS_PAGES: ReadonlyArray<{ id: number; label: string; groups: readonly string[]; wide?: boolean }> = [
  { id: 0, label: 'Machine settings', groups: ['xml'], wide: true },
  { id: 7, label: 'Matrix correction', groups: ['matrix'], wide: true },
  { id: 1, label: 'Checklists', groups: ['checklists'] },
  { id: 2, label: 'Display', groups: ['display'] },
  { id: 3, label: 'Controller & laser', groups: ['controller', 'laser', 'process'] },
  { id: 4, label: 'Axes & homing', groups: ['axes'] },
  { id: 5, label: 'Safety I/O', groups: ['safety'] },
  { id: 6, label: 'Outputs', groups: ['outputs'] },
  { id: 8, label: 'Gas costs', groups: ['gas'] },
];
