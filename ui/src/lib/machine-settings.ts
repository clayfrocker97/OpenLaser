import { field as recipeField, stageOf } from './recipe';
import { quantity } from './units.svelte';

/** Full XML attributes and their independently observed controller words. */
export type XmlField = { path: string; name: string; value: string };
export type Comparison = { address: number; mask: number; expected: number; actual: number | null; fields: string[] };
export type MachineSettings = { name: string; sha256: string; fields: XmlField[]; comparisons: Comparison[]; problem: string | null; connected: boolean };
export const fieldId = (f: Pick<XmlField, 'path' | 'name'>): string => `${f.path}/@${f.name}`;

const labels: Record<string, string> = {
  WritePluse: 'Motor pulses per revolution', SpeedRatio: 'Travel per motor revolution',
  SoftLimitMaxLen: 'Maximum axis travel', GoOriginalDirection: 'Homing direction',
  NegativeType: 'Negative limit input polarity', ForwardType: 'Positive limit input polarity',
  ZoreType: 'Origin input polarity', OriginalInput: 'Origin input port',
  NegativeLimitInput: 'Negative limit input port', ForwardLimitInput: 'Positive limit input port',
  AxisReverse: 'Reverse motor direction', EncoderReverse: 'Reverse encoder direction',
  IsRotatingShaft: 'Rotary axis', EnableZphaseSignal: 'Use encoder index for homing',
  SecondGoHome: 'Second homing pass', SampleType: 'Homing sample signal',
  FastSpeed: 'Fast homing speed', SecondSpeed: 'Slow homing speed', ReturnLength: 'Homing pull-off distance',
  BrakeOutput: 'Brake output port', AccelerationTime: 'Acceleration ramp time',
  DoubleDevice: 'Dual drive enabled', CloseBreakTime: 'Brake closing delay',
  EStop: 'Emergency stop input port', EnableSoftLimit: 'Axis soft limits enabled',
  zfSoftLimitEnable: 'Head soft limits enabled', ZFType: 'Head height controller type',
  ZFGoOriginDone: 'Head homed output port', AFType: 'Optical focus control type',
  NoFollow: 'Disable surface following', NoManu: 'Disable processing',
  NoCloseGasInManu: 'Keep gas on while processing', NoDrill: 'Disable piercing',
  CutPower: 'Cutting duty cycle', CutDuty: 'Cutting duty cycle', CutFreq: 'Cutting frequency',
  ManuType: 'Piercing and height mode', LaserDAPort: 'Laser analog output port',
  LaserDAType: 'Laser analog output type', DOLaserGate: 'Laser shutter output port',
  DOLaser: 'Laser PWM output port', DORedLight: 'Red pointer output port',
  DORemoteStart: 'Laser remote start output port', WaitSignal: 'Idle signal output port',
  ManuSignal: 'Processing signal output port', DIWarning: 'Head warning input port',
  CustomDOStr: 'Custom output definitions', AlarmDIStr: 'Custom alarm input definitions',
  FunctionDIStr: 'Function input definitions', InterpolationCycle: 'Controller interpolation cycle',
  m_iHardwareModel: 'Controller hardware model', m_iEnableLaserType: 'Saved laser mode',
};

/** Humanize unknown fields without changing their literal names or values. */
export function readable(name: string, path = ''): string {
  if (labels[name]) return labels[name];
  if (/^FocusGradualTime\d*$/.test(name)) return `Pierce stage duration alias${name.match(/\d+$/) ? ` ${Number(name.match(/\d+$/)![0]) + 1}` : ''}`;
  if (isLayer(path)) {
    const known = recipeField(name).label;
    if (known !== name) {
      const stage = stageOf(name);
      const prefix = stage !== null ? `Pierce stage ${stage + 1} · ` : name.startsWith('CleanResidue_') ? 'Slag removal · ' : name.startsWith('SmoothPierce') ? 'Smooth piercing · ' : name.startsWith('UD_Up') ? 'Cut start · ' : name.startsWith('UD_Down') ? 'Cut end · ' : '';
      return `${prefix}${known}`;
    }
  }
  const text = name.replace(/^m_[ibd]/, '').replace(/CO2/g, 'CO₂ ').replace(/ZF/g, 'Head ').replace(/AF(?=[A-Z])/, 'Focus ')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2').replace(/([a-z])([A-Z0-9])/g, '$1 $2').replace(/_/g, ' ')
    .replace(/\bManu\b/g, 'Processing').replace(/\bDrill\b/g, 'Pierce').replace(/\bMico\b/g, 'Micro').replace(/\bRoate\b/g, 'Rotate')
    .replace(/\bEncolse\b/g, 'Enclosed').replace(/\bDecc\b/g, 'Deceleration').replace(/\bAcc\b/g, 'Acceleration')
    .replace(/\bDcc\b/g, 'Deceleration').replace(/\bFreq\b/g, 'Frequency').replace(/\bVel\b/g, 'Speed')
    .replace(/\bLen\b/g, 'Length').replace(/\bPos\b/g, 'Position').replace(/\bPt\b/g, 'Point')
    .replace(/\bDI\b/g, 'Input').replace(/\bDO\b/g, 'Output').replace(/\bDA\b/g, 'Analog output')
    .replace(/\bStr\b/g, 'Definitions').replace(/\s+/g, ' ').trim();
  return text.charAt(0).toUpperCase() + text.slice(1);
}

const groups: Record<string, string> = {
  PAxisParam: 'Axis routing', PHomeParam: 'Homing defaults', PMachineAxisConfig: 'Drive coordination',
  PZFParam: 'Head height control', PLaserParam: 'Laser & lifting table', PDOParam: 'Digital outputs',
  PDIParam: 'Inputs & alarms', PDAParam: 'Analog calibration', PManuParam: 'Machine operation',
  PFCParam: 'Motion planning', PAFParam: 'Optical focus data', PECParam: 'Jog & head limits',
  PSoftParam: 'Software & controller', PGraphParam: 'Geometry & processing', PNestParam: 'Nesting',
  PImportGraphParam: 'File import & display', PGasParam: 'Gas & pressure',
};
export function groupName(path: string): string {
  const name = path.split('/')[2] ?? '';
  const axis = name.match(/^PMachineAxisConfig_(\d)$/);
  if (axis) return `${['X axis', 'Y axis', 'Auxiliary axis 2', 'Z head axis', 'W table axis'][Number(axis[1])] ?? 'Axis'} settings`;
  const layer = name.match(/^P(CO2)?LayerParam(\d+)$/);
  if (layer) return `${layer[1] ? 'CO₂' : 'Fiber'} layer ${layer[2]}`;
  return groups[name] ?? readable(name.replace(/^P/, ''));
}
export const groupKey = (path: string): string => path.split('/').slice(0, 3).join('/');
export const isLayer = (path: string): boolean => /\/P(CO2)?LayerParam/.test(path);

// Explicit source-unit inventory. Do not infer units from substrings: a port,
// ratio, encoder word, or percentage can also contain "Speed" or "Length".
const unitFields: Record<string, Record<string, string[]>> = {
  PAxisParam: { mm: ['MaxLength', 'DoubleDriverToleranceLength'] },
  PHomeParam: { mm: ['HomeOffset'], 'mm/s': ['FastSpeed', 'SlowSpeed', 'WorkSpeed', 'IdelSpeed'], 'mm/s²': ['Acc'], ms: ['AccTime_ms'] },
  PMachineAxisConfig_axis: {
    mm: ['SoftLimitMaxLen', 'ReturnLength'], 'mm/rev': ['SpeedRatio'],
    'mm/s': ['FastSpeed', 'SecondSpeed'], 'mm/s²': ['Acceleration'], ms: ['AccelerationTime'],
  },
  PZFParam: { mm: ['ZFDockHeight'], 'mm/s': ['ZFFollowSpeed', 'ZFJogSpeed', 'ZFFastJogSpeed', 'ZFUpSpeed'] },
  PManuParam: {
    mm: ['XAxisGapCompensate', 'YAxisGapCompensate', 'VerCorrectABLength', 'VerCorrectACLength', 'VerCorrectL1Length', 'VerCorrectL2Length', 'DirectDrillMaxHeight', 'DirectSecondDrillMaxHeight', 'PlatformExchangeLength', 'ZFSafeHeight', 'SingleRollSheetLength', 'ManuCrashProtectUpHeight', 'EnableManuCrashProtectMinHeight', 'SingleForwardRollLength', 'SingleBackwardLength', 'StepLength', 'ForwardBackwardLength', 'ResumeBackLength', 'ContourShiftXDist', 'ContourShiftYDist', 'ShortNoUpMaxLength'],
    'mm/s': ['PlatformExchangeSpeed', 'SingleRollSheetSpeed', 'ECAxisMaxSpeed', 'JogFastSpeed', 'JogSlowSpeed', 'ForwardBackwardSpeed', 'BoundSpeed', 'XFastMoveSpeed'],
    'mm/s²': ['XFastMoveAcc', 'ManuAcc'], bar: ['DefaultGasPressure'],
    ms: ['AccTime', 'EmptyMoveAccTime', 'PtLaserTime_ms', 'GasDelay', 'DirectGasDelay', 'ChangeGasDelay', 'iProportionalOpenSleep'],
  },
  PGasParam: { bar: ['DAMaxPressure', 'NewDAMAxPressureAir', 'NewDAMAxPressureO2', 'NewDAMAxPressureN2'] },
  PDOParam: { mm: ['VFDMotorSlowStartLength', 'VFDMotorSlowStopLength'] },
  PFCParam: { mm: ['FrogJumpMinHeight'], 'mm/s': ['MaxSpeed'], 'mm/s²': ['MaxAcc'] },
  PSoftParam: {
    mm: ['InterfereTotalLength', 'InterfereStepLength', 'InterfereGapAdjust'],
    'mm/s': ['InterfereSpeed', 'MicoLinkSlowDownVel'],
  },
  PAFParam: { mm: ['OriginOffset'], 'mm/s': ['AFPosSpeed'] },
  PECParam: { mm: ['ECStepLength'], 'mm/s': ['ECFastSpeed'] },
  PGraphParam: {
    mm: ['GuideLineLength', 'GuideArcRadius', 'LoopGapOverCutLength', 'MicroLinkLength', 'RowGapVct', 'ColGapVct', 'AutoMicroLinkStep', 'MinEnableMicoLinkGraphSize', 'BridgeWidth', 'SmoothAccuracy', 'OffsetDist', 'ArcRoundRadius', 'UnloadAngleRadius', 'AlphaMinEdgeLen', 'AlphaLen', 'CircleFlyMaxLen', 'LineFlyCollineTol', 'LineFlyMaxLinkLen', 'LineFlyMaxLen', 'FillCircleRadius', 'FillCircleStock', 'FillCircleSpace', 'MaxSizeError', 'MinSizeError', 'StdSheetHeight', 'StdSheetWidth', 'OverlapGate', 'ManualConnectGate', 'BallArmGuideLineLength', 'BallArmCircleRadius', 'AdvTextOffset', 'BrushUpZVal', 'BrushDiveHeight', 'CleanStartXPos', 'CleanStartYPos', 'CleanMoveLength', 'EdgeSeekFollowHeight', 'EdgeSeekUpHeight', 'FastEdgeSeekMoveOutTolerance', 'SlowEdgeSeekMoveOutTolerance', 'EdgeSeekXYPointDist', 'EdgeOffsetX', 'EdgeOffsetY', 'EdgeSeekMoveInDestHeight', 'EdgeSeekSafeUpHeight', 'DualServoCalibMaxLength', 'DualServoCalibAdjustOffset', 'DualServoCalibAdjustTolerance', 'RotatePlatformManuPtStepLength0', 'RotatePlatformManuPtStepLength1', 'AfterCleanZFCalibXPos', 'AfterCleanZFCalibYPos', 'PerRollSheetOffset', 'BeforeManuDockPtX', 'BeforeManuDockPtY', 'RollSheetWidth', 'CutOffRollSheetX_LeftEdge', 'CutOffRollSheetX_RightEdge', 'RollSheetWidth_btnCmd', 'CutOffOutEdgeCheckTol', 'CutOffHeadUpHWorkDone', 'BatchCutX_LeftEdge', 'BatchCutX_RightEdge', 'EdgeSeekXPointDist', 'EdgeSeekYPointDist', 'EdgeBoardSizeX', 'EdgeBoardSizeY', 'EdgeSeekStartPointX', 'EdgeSeekStartPointY', 'minHorizontalLineGap', 'minScanLineLength', 'scanSideLineLength'],
    'mm/s': ['CleanMoveSpeed', 'EdgeSeekDownSpeed', 'EdgeSeekXYFastSpeed', 'EdgeSeekXYSlowSpeed', 'CutOffRollSheetSpeed', 'EdgeSeekOutSpeed'],
  },
  PNestParam: { mm: ['SheetWidth', 'SheetHeight', 'EdgeStock', 'PartSpace', 'ShareEdgeMinLen', 'NestAccuracy'] },
  PImportGraphParam: { mm: ['MicoGraphGate', 'OverlapGate', 'ConnectGate', 'SmoothAccuracy', 'KeyboardMoveStepLength'] },
};

export function fieldUnit(f: Pick<XmlField, 'path' | 'name'>, fields: XmlField[] = []): string {
  if (isLayer(f.path)) return recipeField(f.name).unit ?? '';
  const group = f.path.split('/')[2] ?? '';
  const axis = /^PMachineAxisConfig_\d+$/.test(group);
  const source = Object.entries(unitFields[axis ? 'PMachineAxisConfig_axis' : group] ?? {}).find(([, names]) => names.includes(f.name))?.[0] ?? '';
  // A rotary axis is angular; an imperial preference must not scale it.
  if (axis && fields.some(other => other.path === f.path && other.name === 'IsRotatingShaft' && Number(other.value) !== 0)) {
    return ({ mm: '°', 'mm/s': '°/s', 'mm/s²': '°/s²', 'mm/rev': '°/rev' } as Record<string, string>)[source] ?? source;
  }
  return source;
}

export function xmlValue(f: Pick<XmlField, 'path' | 'name'>, value: string, fields: XmlField[] = []): string {
  const unit = fieldUnit(f, fields);
  return unit && value.trim() && Number.isFinite(Number(value)) ? quantity(value, unit) : value || '(empty)';
}

/** Split each large layer bank into the same process families as the recipe editor. */
export function sectionName(f: XmlField): string {
  if (!isLayer(f.path)) return groupName(f.path);
  const stage = stageOf(f.name);
  if (stage !== null) return `Piercing · stage ${stage + 1}`;
  if (/^(SmoothPierce|EnableSmoothPierce)/.test(f.name)) return 'Smooth piercing';
  if (/^(PreDrill|AfterPreDrill)/.test(f.name)) return 'Batch piercing';
  if (/^CleanResidue_/.test(f.name)) return 'Slag removal';
  if (/^(UD_Up|UD_Down|SlowStart)/.test(f.name)) return 'Cut start & end';
  if (/Curve|AdjustWithSpeed/.test(f.name)) return 'Power & frequency curves';
  if (/^(Cut|AdvFixHeight|ManuType|LayerFileName|Note|NoManu)/.test(f.name)) return 'Cutting';
  if (/Laser.*Delay|Gas|UpHeight|ShortDist|NoFollow/.test(f.name)) return 'Height, timing & gas';
  return 'Other process settings';
}

/**
 * A parameter that is plainly on or off: its name says so (Enable…, Is…,
 * Use…, …Reverse) and it holds 0 or 1. Other 0/1 values are often types or
 * port numbers, so they keep the number editor.
 */
export const isSwitch = (field: XmlField, value: string): boolean =>
  (value === '0' || value === '1') && /Enable|^(m_i|m_b)?(Is|Use)[A-Z]|Reverse$/.test(field.name);
