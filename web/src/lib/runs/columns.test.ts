import { describe, expect, it } from 'vitest';

import type { RunSummary } from '#lib/api/types.js';
import { BASE_COLUMNS, compareRuns, sortValue } from './columns.js';

function run(name: string, checkpoints: number, bytes: number): RunSummary {
	return {
		id: `01a18923-ac7d-70fc-8a16-cd20a829836${name.length}`,
		name,
		state: 'finished',
		group_name: null,
		tags: [],
		aliases: [],
		created_at_ms: 0,
		start_ms: 0,
		archived_at_ms: null,
		last_step: null,
		metric_min: {},
		metric_max: {},
		abort_requested_at_ms: null,
		renames: {},
		rename_scales: {},
		checkpoints,
		checkpoint_bytes: bytes,
		heartbeat_at_ms: 0,
		finished_at_ms: null,
		summary: {},
		config: {}
	};
}

describe('checkpoints column', () => {
	it('is a run column, sorting by size with runs without checkpoints last', () => {
		expect(BASE_COLUMNS).toContain('checkpoints');
		const runs = [run('none', 0, 0), run('small', 3, 10), run('large', 1, 5_000)];
		expect(sortValue(runs[0], 'checkpoints')).toBeUndefined();
		const order = (desc: boolean) =>
			runs.toSorted(compareRuns('checkpoints', desc)).map((r) => r.name);
		expect(order(true)).toEqual(['large', 'small', 'none']);
		expect(order(false)).toEqual(['small', 'large', 'none']);
	});
});
