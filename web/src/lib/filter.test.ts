import { describe, expect, it } from 'vitest';

import type { RunSummary } from '#lib/api/types.js';
import { FilterError, matches, parseFilter } from './filter.js';

function run(overrides: Partial<RunSummary>): RunSummary {
	return {
		id: '01a18923-ac7d-70fc-8a16-cd20a8298365',
		name: 'brave-falcon-1',
		state: 'finished',
		group_name: null,
		tags: [],
		aliases: [],
		created_at_ms: 0,
		start_ms: 0,
		archived_at_ms: null,
		last_step: null,
		heartbeat_at_ms: 0,
		finished_at_ms: null,
		summary: {},
		config: {},
		...overrides
	};
}

const baseline = run({
	name: 'baseline-lr3',
	tags: ['baseline', 'v2'],
	aliases: ['best'],
	group_name: 'sweep-a',
	config: { lr: 1e-3, optimizer: { name: 'AdamW', betas: [0.9, 0.99] }, amp: true },
	summary: { 'val/acc': 0.91, loss: 0.2 },
	last_step: 9_999
});
const crashed = run({
	name: 'wide-net',
	state: 'crashed',
	config: { lr: 1e-2, optimizer: { name: 'SGD' } },
	summary: { 'val/acc': 0.5 }
});

const select = (filter: string) =>
	[baseline, crashed].filter((r) => matches(parseFilter(filter), r)).map((r) => r.name);

describe('filters', () => {
	it('match everything when empty', () => {
		expect(parseFilter('  ')).toBeNull();
		expect(select('')).toEqual(['baseline-lr3', 'wide-net']);
	});

	it('compare numbers in config and summary', () => {
		expect(select('config.lr < 1e-2')).toEqual(['baseline-lr3']);
		expect(select('config.lr <= 1e-2')).toEqual(['baseline-lr3', 'wide-net']);
		expect(select('summary.val/acc >= 0.9')).toEqual(['baseline-lr3']);
		expect(select('lr = 0.01')).toEqual(['wide-net']);
		expect(select('loss > 0')).toEqual(['baseline-lr3']);
	});

	it('read nested config keys with dots', () => {
		expect(select('config.optimizer.name = adamw')).toEqual(['baseline-lr3']);
		expect(select('config.optimizer.name:sg')).toEqual(['wide-net']);
		expect(select('config.amp = true')).toEqual(['baseline-lr3']);
	});

	it('match tags, aliases and states', () => {
		expect(select('tag:baseline')).toEqual(['baseline-lr3']);
		expect(select('tag:base'), 'tags match whole').toEqual([]);
		expect(select('tag != baseline')).toEqual(['wide-net']);
		expect(select('alias:best')).toEqual(['baseline-lr3']);
		expect(select('state:crashed')).toEqual(['wide-net']);
		expect(select('group:sweep')).toEqual(['baseline-lr3']);
	});

	it('combine with and, or, not and parentheses', () => {
		expect(select('config.lr < 1 and not state:crashed')).toEqual(['baseline-lr3']);
		expect(select('config.lr < 1 state:crashed'), 'implicit and').toEqual(['wide-net']);
		expect(select('state:crashed or tag:v2')).toEqual(['baseline-lr3', 'wide-net']);
		expect(select('not (state:crashed or tag:v2)')).toEqual([]);
		expect(select('tag:v2 AND (lr > 1 OR loss < 1)')).toEqual(['baseline-lr3']);
	});

	it('search names, groups, tags and aliases with bare words', () => {
		expect(select('wide')).toEqual(['wide-net']);
		expect(select('"sweep-a"')).toEqual(['baseline-lr3']);
		expect(select('BEST')).toEqual(['baseline-lr3']);
	});

	it('count steps', () => {
		expect(select('steps >= 10000')).toEqual(['baseline-lr3']);
		expect(select('steps < 10'), 'no steps logged is missing, not 0').toEqual([]);
	});

	it('treat missing values as not matching', () => {
		expect(select('summary.loss < 1')).toEqual(['baseline-lr3']);
		expect(select('summary.loss != 0.2')).toEqual(['wide-net']);
		expect(select('config.amp = null')).toEqual(['wide-net']);
		expect(select('name < 5'), 'ordering is numeric only').toEqual([]);
	});

	it('match regular expressions', () => {
		expect(select('name ~ "^b.*\\d$"')).toEqual(['baseline-lr3']);
		expect(() => parseFilter('name ~ "("')).toThrow('invalid regular expression');
	});

	it('report syntax errors with a position', () => {
		const at = (filter: string) => {
			try {
				parseFilter(filter);
			} catch (err) {
				return [(err as FilterError).message, (err as FilterError).at];
			}
			return null;
		};
		expect(at('lr <')).toEqual(['expected a value after "<"', 4]);
		expect(at('(tag:a')).toEqual(['missing ")"', 6]);
		expect(at('"open')).toEqual(['unclosed quote', 0]);
		expect(at('tag:a or')).toEqual(['expected a condition', 8]);
		expect(at('a )')).toEqual(['unexpected input', 2]);
	});
});
