// Problems with the machine files, shown as alarms: they block cutting just
// like a controller alarm, and the fix is always in Settings → Machine.
import type { Document } from '../api';

/** One machine-files problem, worded for the Alarms dialog. */
export interface SetupAlarm { title: string; fix: string; detail: string }

const MACHINE_FILES = /machine files|machine backup|missing setting|\bParam\./;

export function setupAlarms(doc: Document | null | undefined): SetupAlarm[] {
  if (!doc) return [];
  const found: SetupAlarm[] = [];
  const fix = 'Import the full machine backup in Settings → Machine.';
  if (doc.bindings_error) found.push({ title: 'Machine backup not loaded', fix, detail: doc.bindings_error });
  const draftError = doc.draft?.error;
  if (draftError && MACHINE_FILES.test(draftError) && draftError !== doc.bindings_error) {
    found.push({ title: 'Machine backup is missing a setting', fix, detail: draftError });
  }
  return found;
}

/** Whether a refusal comes from the machine files, so the status line can name the alarm. */
export const isSetupAlarm = (reason: string | null | undefined): boolean => !!reason && MACHINE_FILES.test(reason);
