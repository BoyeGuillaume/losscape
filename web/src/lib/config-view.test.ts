import { describe, expect, it } from 'vitest';
import { stringify } from 'yaml';

import { countLeaves, filterConfig, splitMatches, type Json } from './config-view.js';

const config: Json = {
	lr: 0.001,
	optimizer: { name: 'AdamW', betas: [0.9, 0.999], weight_decay: 0.01 },
	model: { depth: 12, blocks: [{ kind: 'attention' }, { kind: 'mlp' }] },
	notes: 'warmup then cosine'
};

describe('filterConfig', () => {
	it('keeps everything without a query', () => {
		expect(filterConfig(config, '  ')).toBe(config);
	});

	it('keeps whole subtrees under matching keys', () => {
		expect(filterConfig(config, 'optim')).toEqual({ optimizer: config.optimizer });
	});

	it('keeps matching values with the keys leading to them', () => {
		expect(filterConfig(config, 'adam')).toEqual({ optimizer: { name: 'AdamW' } });
		expect(filterConfig(config, 'mlp')).toEqual({ model: { blocks: [{ kind: 'mlp' }] } });
		expect(filterConfig(config, '0.999')).toEqual({ optimizer: { betas: [0.9, 0.999] } });
		expect(filterConfig(config, 'COSINE')).toEqual({ notes: 'warmup then cosine' });
	});

	it('gives nothing when nothing matches', () => {
		expect(filterConfig(config, 'nope')).toBeUndefined();
	});
});

describe('helpers', () => {
	it('counts leaves; lists of numbers are one value', () => {
		expect(countLeaves(config)).toBe(8);
	});

	it('splits text around matches', () => {
		expect(splitMatches('AdamWadam', 'adam')).toEqual([
			{ text: 'Adam', match: true },
			{ text: 'W', match: false },
			{ text: 'adam', match: true }
		]);
	});

	it('writes YAML that keeps ambiguous strings as strings', () => {
		const yaml = stringify({ a: 'true', b: '1e-3', c: 'x: y', d: true, e: 0.001 });
		expect(yaml).toBe('a: "true"\nb: "1e-3"\nc: "x: y"\nd: true\ne: 0.001\n');
	});
});
