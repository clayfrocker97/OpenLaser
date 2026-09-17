// Physical values stay in their source units. Only presentation and entry convert.
export type UnitSystem = 'metric' | 'imperial';
type Conversion = { unit: string; factor: number; offset?: number; digits?: number };

const imperial: Record<string, Conversion> = {
  mm: { unit: 'in', factor: 1 / 25.4, digits: 4 },
  'mm²': { unit: 'in²', factor: 1 / 25.4 ** 2, digits: 3 },
  'mm³': { unit: 'in³', factor: 1 / 25.4 ** 3, digits: 3 },
  'mm/s': { unit: 'in/min', factor: 60 / 25.4, digits: 3 },
  'mm/min': { unit: 'in/min', factor: 1 / 25.4, digits: 3 },
  'm/min': { unit: 'in/min', factor: 1000 / 25.4, digits: 3 },
  'mm/s²': { unit: 'in/s²', factor: 1 / 25.4, digits: 3 },
  'mm/s³': { unit: 'in/s³', factor: 1 / 25.4, digits: 3 },
  'mm/rev': { unit: 'in/rev', factor: 1 / 25.4, digits: 5 },
  'counts/mm': { unit: 'counts/in', factor: 25.4, digits: 2 },
  'pulses/mm': { unit: 'pulses/in', factor: 25.4, digits: 2 },
  'units/mm': { unit: 'units/in', factor: 25.4, digits: 2 },
  'µm': { unit: 'in', factor: 1 / 25400, digits: 6 },
  'μm': { unit: 'in', factor: 1 / 25400, digits: 6 },
  'cm': { unit: 'in', factor: 1 / 2.54, digits: 4 },
  'm': { unit: 'ft', factor: 1 / 0.3048, digits: 3 },
  'm²': { unit: 'ft²', factor: 1 / 0.3048 ** 2, digits: 3 },
  bar: { unit: 'psi', factor: 100000 / 6894.757293168, digits: 2 },
  MPa: { unit: 'psi', factor: 1000000 / 6894.757293168, digits: 2 },
  kPa: { unit: 'psi', factor: 1000 / 6894.757293168, digits: 2 },
  '°C': { unit: '°F', factor: 1.8, offset: 32, digits: 1 },
};

function remembered(): UnitSystem {
  try { return localStorage.getItem('ol-units') === 'imperial' ? 'imperial' : 'metric'; }
  catch { return 'metric'; }
}

class DisplayUnits {
  system = $state<UnitSystem>(remembered());
  set(system: UnitSystem): void {
    this.system = system;
    try { localStorage.setItem('ol-units', system); } catch { /* session preference still works */ }
  }
}
export const units = new DisplayUnits();

function conversion(source: string, system: UnitSystem): Conversion {
  return system === 'imperial' && imperial[source] ? imperial[source] : { unit: source, factor: 1 };
}

export const unitLabel = (source: string, system = units.system): string => conversion(source, system).unit;
export const toDisplay = (value: number, source: string, system = units.system): number => {
  const { factor, offset = 0 } = conversion(source, system);
  return value * factor + offset;
};
export const toSource = (value: number, source: string, system = units.system): number => {
  const { factor, offset = 0 } = conversion(source, system);
  return (value - offset) / factor;
};

/** Enough decimals to inspect a small kerf or a fine coordinate in inches. */
export function displayNumber(value: number | string | null | undefined, source: string, digits = 2): string {
  if (value === null || value === undefined || value === '') return '—';
  const n = Number(value);
  if (!Number.isFinite(n)) return String(value);
  const places = Math.max(digits, conversion(source, units.system).digits ?? digits);
  const shown = toDisplay(n, source);
  // Never label a nonzero setting as zero because the display rounded it away.
  const precise = shown !== 0 && Math.abs(shown) < 10 ** -places ? Math.min(9, Math.ceil(-Math.log10(Math.abs(shown))) + 1) : places;
  return shown.toFixed(precise).replace(/^-0(?:\.0+)?$/, (zero) => zero.slice(1));
}

export function quantity(value: number | string | null | undefined, source: string, digits = 2): string {
  const shown = displayNumber(value, source, digits);
  return shown === '—' ? shown : `${shown}${unitLabel(source) ? ` ${unitLabel(source)}` : ''}`;
}
export const distance = (value: number, digits = 2): string => displayNumber(value, 'mm', digits);

/** Convert explicitly unit-labelled numbers in machine-generated diagnostics. */
export function diagnosticText(text: string): string {
  if (units.system === 'metric') return text;
  return text.replace(/(?<![\w./\\])(-?\d+(?:[ _]\d{3})*(?:\.\d+)?)(?:\s*([–−]|to)\s*(-?\d+(?:[ _]\d{3})*(?:\.\d+)?))?\s+(mm\/s²|mm\/s|mm\/min|mm²|mm|bar|°C)(?![\w/])/g,
    (_match, first: string, separator: string | undefined, last: string | undefined, source: string) => {
      const number = (value: string) => displayNumber(Number(value.replace(/[ _]/g, '')), source).replace(/(\.\d*?)0+$/, '$1').replace(/\.$/, '');
      return `${number(first)}${last ? ` ${separator} ${number(last)}` : ''} ${unitLabel(source)}`;
    });
}

/** Editable values keep more precision than compact labels. */
export function inputValue(value: number, source: string, system = units.system): string {
  return String(Number(toDisplay(value, source, system).toPrecision(12)));
}

/** Tapping Done on the displayed value preserves every original metric digit. */
export function sourceInput(text: string, original: number, source: string, system = units.system): number {
  const parsed = text.trim() ? Number(text) : NaN;
  if (!Number.isFinite(parsed)) return NaN;
  return parsed === Number(inputValue(original, source, system)) ? original : toSource(parsed, source, system);
}
