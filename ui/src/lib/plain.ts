// Plain-language status. The server and controller explain refusals in
// precise, technical words ("missing setting ManuParam.FC.ShortNoUpMaxLength",
// "no reply to read of register 1000"). Operators read one plain line; the
// original text stays available as a detail on tap.
import type { AlarmView, Gate } from '../api';
import { diagnosticText } from './units.svelte';

/** A reason in plain words, with the original kept when it was reworded. */
export interface Plain {
  text: string;
  /** The original technical text, when `text` rewords it. */
  detail: string | null;
}

const sentence = (text: string): string => {
  const trimmed = text.trim();
  if (!trimmed) return '';
  const first = trimmed[0]!.toUpperCase() + trimmed.slice(1);
  return /[.!?…]$/.test(first) ? first : `${first}.`;
};

/** Refusals whose own words are already plain, reworded only lightly. */
const EXACT: Record<string, string> = {
  'connect the machine first': 'Connect the machine first.',
  'the machine is not connected': 'Connect the machine first.',
  'home XY first': 'Home the machine first.',
  'open a part first': 'Open a part first.',
  'choose a material first': 'Choose a material first.',
  'set a job origin first': 'Set the job origin first.',
  'no head controller': 'This machine has no height-sensing head.',
  'no manual Z control configured': 'Manual head moves are not set up for this machine.',
  'no program is running': 'Nothing is running.',
  'nothing is held': 'Nothing is paused.',
  'already connected': 'Already connected.',
  'connecting': 'Connecting…',
  'an operation is being prepared': 'Please wait: the machine is getting ready.',
  'another operation is active': 'Please wait for the current move to finish.',
  'controller alarms are active': 'An alarm is active. Tap for details.',
  'the machine is not configured': 'Machine files not loaded. Import the machine backup in Settings.',
  'the machine files are not bound': 'Machine files not loaded. Import the machine backup in Settings.',
  'no machine files': 'Machine files not loaded. Import the machine backup in Settings.',
  'the controller task has stopped': 'OpenLaser lost the controller. Reconnect.',
};

/** Technical patterns, in order; the first match wins. */
const PATTERNS: Array<[RegExp, (match: RegExpMatchArray) => string]> = [
  [/^alarms are active: (.+)$/, (m) => {
    const names = m[1]!.split('; ');
    return names.length === 1 ? `Alarm: ${names[0]}.` : `${names.length} alarms: ${names[0]} and ${names.length - 1} more.`;
  }],
  [/^(.+) is active$/, (m) => `Please wait: ${m[1]!.toLowerCase()} is in progress.`],
  [/machine files have no \S+/, () => 'Alarm: machine backup is missing a setting. Tap for details.'],
  [/missing setting \S+/, () => 'The machine files are missing a setting. Re-import the machine backup in Settings.'],
  [/^setting \S+: /, () => 'A machine setting is not valid. Check the machine backup in Settings.'],
  [/no reply to \S+ of register/, () => 'The controller did not answer. Check its power and network cable.'],
  [/^socket: /, () => 'Could not open a network connection to the controller.'],
  [/^bad reply: /, () => 'The controller sent an unexpected reply. Reconnect.'],
  [/^XML /, () => 'The machine backup file could not be read.'],
  [/machine files did not bind/, () => 'Connected, but the machine files are not loaded. Import the machine backup in Settings.'],
];

/** A vendor parameter path such as `ManuParam.FC.ShortNoUpMaxLength`. */
const PARAMETER = /\b[A-Z]\w*Param\.[\w.]+/g;
/** A raw register or bit reference that means nothing to an operator. */
const REGISTER = /\b(controller group \d+ bit \d+|axis \d+ detail bit \d+|head bit \d+|register \d+)\b/;

/** The fix every row shares when homing the head clears them all. */
const HOME_HEAD = /^Home the head\b/;

/**
 * One refusal or error in plain words. With the active `alarms`, a refusal
 * caused by alarms that one action clears names that action instead.
 */
export function plain(reason: string | null | undefined, alarms: AlarmView[] = []): Plain {
  const raw = (reason ?? '').trim();
  if (!raw) return { text: '', detail: null };
  const blocking = alarms.filter((alarm) => alarm.blocking);
  if (raw.startsWith('alarms are active') && blocking.length > 0 && blocking.every((alarm) => HOME_HEAD.test(alarm.fix))) {
    return { text: 'Head needs homing. Tap for details.', detail: null };
  }
  const exact = EXACT[raw];
  if (exact) return { text: exact, detail: null };
  for (const [pattern, word] of PATTERNS) {
    const match = raw.match(pattern);
    if (match) {
      const text = word(match);
      // Alarm names are already plain; everything else keeps its original.
      return { text: diagnosticText(text), detail: pattern.source.startsWith('^alarms') ? null : raw };
    }
  }
  if (raw.search(PARAMETER) >= 0 || REGISTER.test(raw)) {
    const text = raw.replace(PARAMETER, 'a machine setting').replace(REGISTER, 'a controller signal');
    return { text: sentence(diagnosticText(text)), detail: raw };
  }
  return { text: sentence(diagnosticText(raw)), detail: null };
}

/** A control and whether it is available now. */
export type NamedGate = [name: string, gate: Gate];

/** Why each unavailable control is unavailable, grouped by reason. */
export function blockedControls(gates: NamedGate[]): Array<{ names: string[]; reason: Plain }> {
  const byReason = new Map<string, string[]>();
  for (const [name, gate] of gates) {
    if (gate.ok || !gate.reason) continue;
    byReason.set(gate.reason, [...(byReason.get(gate.reason) ?? []), name]);
  }
  return [...byReason].map(([reason, names]) => ({ names, reason: plain(reason) }));
}

/** Alarm rows grouped by the one action that clears them. */
export interface AlarmGroup {
  /** The rows the action relieves, blocking ones first. */
  alarms: AlarmView[];
  /** The relief shared by every row in the group. */
  relief: AlarmView['relief'];
}

/**
 * Head homing clears several rows at once ("Head needs homing", "Head
 * fault"); those share one card and one Home head button. Every other row
 * keeps its own card.
 */
export function groupAlarms(alarms: AlarmView[]): AlarmGroup[] {
  const homes = alarms.filter((alarm) => alarm.relief.moves_axes);
  const joins = (alarm: AlarmView) => homes.length > 0 && (alarm.relief.moves_axes || HOME_HEAD.test(alarm.fix));
  // The rows that offer homing name the card.
  const homing = [...homes, ...alarms.filter((alarm) => joins(alarm) && !alarm.relief.moves_axes)];
  const groups: AlarmGroup[] = homing.length ? [{ alarms: homing, relief: homes[0]!.relief }] : [];
  for (const alarm of alarms) if (!joins(alarm)) groups.push({ alarms: [alarm], relief: alarm.relief });
  return groups;
}

/** The distinct plain names of a set of rows, in order. */
export const alarmTitles = (alarms: AlarmView[]): string[] => [...new Set(alarms.map((alarm) => alarm.title))];
