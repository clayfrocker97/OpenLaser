import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { PreflightReview } from '../api';
import { api } from '../api/client';
import { beginRun } from './run-actions';

vi.mock('../api/client', () => ({ api: {
  placement:vi.fn(), preparePlacement:vi.fn(), preflight:vi.fn(), machine:vi.fn(),
} }));

const finished = { execution:{ frame:false }, machine:{ program:{ state:'completed' } } };
const idle = { execution:null, machine:{ program:null } };
const review:PreflightReview = { token:'review-token', intent:'run', steps:[], satisfied:[], gases:[], confirm_gas:false, dry_run:true };

describe('shared Start workflow', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(api.preflight).mockResolvedValue(review);
  });

  it.each(['completed', 'stopped'])('waits for a %s job reset before positioning or showing preflight', async (state) => {
    let release!:() => void;
    vi.mocked(api.placement).mockReturnValue(new Promise(resolve => { release = () => resolve({ok:true,draft:null,draft_revision:1}); }));
    const checks = { ...review, confirm_gas:true };
    vi.mocked(api.preflight).mockResolvedValue(checks);
    const start = beginRun('run', { ...finished, machine: { program: { state } } });
    expect(api.placement).toHaveBeenCalledWith({kind:'new_run'});
    expect(api.preparePlacement).not.toHaveBeenCalled();
    expect(api.machine).not.toHaveBeenCalled();
    release();
    expect(await start).toEqual(checks);
    expect(api.preparePlacement).toHaveBeenCalledOnce();
    expect(api.machine).not.toHaveBeenCalled();
  });

  it('does not position or start when the completed-job reset is refused', async () => {
    vi.mocked(api.placement).mockRejectedValue(new Error('machine busy'));
    await expect(beginRun('run',finished)).rejects.toThrow('machine busy');
    expect(api.preparePlacement).not.toHaveBeenCalled();
    expect(api.preflight).not.toHaveBeenCalled();
    expect(api.machine).not.toHaveBeenCalled();
  });

  it('keeps first starts and completed frames out of the next-sheet reset', async () => {
    for (const source of [idle,{ ...finished,execution:{frame:true} }]) {
      await beginRun('run',source);
    }
    expect(api.placement).not.toHaveBeenCalled();
    expect(api.machine).toHaveBeenCalledTimes(2);
    expect(api.machine).toHaveBeenCalledWith('run',{preflight:{token:review.token,checked:[],gas_ready:false}});
  });

  it('resumes the selected recovery without resetting or repositioning the job', async () => {
    vi.mocked(api.preflight).mockResolvedValue({...review,intent:'resume'});
    await beginRun('resume',finished);
    expect(api.placement).not.toHaveBeenCalled();
    expect(api.preparePlacement).not.toHaveBeenCalled();
    expect(api.machine).toHaveBeenCalledWith('resume',{preflight:{token:review.token,checked:[],gas_ready:false}});
  });
});
