import { describe, expect, it } from 'vitest';
import { stringify } from 'yaml';

import {
	compact,
	countLeaves,
	filterConfig,
	splitKeyMatches,
	splitMatches,
	type Json
} from './config-view.js';

const config: Json = {
	lr: 0.001,
	optimizer: { name: 'AdamW', betas: [0.9, 0.999], weight_decay: 0.01 },
	model: { depth: 12, blocks: [{ kind: 'attention' }, { kind: 'mlp' }] },
	notes: 'warmup then cosine'
};

/** A list with only its second item (filtered lists keep their indices). */
const second = (item: Json): Json[] => {
	const out: Json[] = [];
	out[1] = item;
	return out;
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
		expect(filterConfig(config, 'mlp')).toEqual({ model: { blocks: second({ kind: 'mlp' }) } });
		expect(filterConfig(config, '0.999')).toEqual({ optimizer: { betas: [0.9, 0.999] } });
		expect(filterConfig(config, 'COSINE')).toEqual({ notes: 'warmup then cosine' });
	});

	it('matches dotted paths across keys', () => {
		expect(filterConfig(config, 'optimizer.be')).toEqual({ optimizer: { betas: [0.9, 0.999] } });
		const found = filterConfig(config, 'model.blocks.1.kind');
		expect(found).toEqual({ model: { blocks: second({ kind: 'mlp' }) } });
		expect(compact(found!)).toEqual({ model: { blocks: [{ kind: 'mlp' }] } });
		expect(filterConfig(config, 'mizer.weight')).toEqual({ optimizer: { weight_decay: 0.01 } });
		expect(filterConfig(config, 'model.name')).toBeUndefined();
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

	it('highlights the parts of keys a dotted query matches', () => {
		expect(splitKeyMatches(['optimizer'], 'mizer.be')).toEqual([
			{ text: 'opti', match: false },
			{ text: 'mizer', match: true }
		]);
		expect(splitKeyMatches(['optimizer', 'betas'], 'mizer.be')).toEqual([
			{ text: 'be', match: true },
			{ text: 'tas', match: false }
		]);
		expect(splitKeyMatches(['model'], 'mizer.be')).toEqual([{ text: 'model', match: false }]);
		expect(splitKeyMatches(['a', 'b', 'c'], 'a.b.c')[0]).toEqual({ text: 'c', match: true });
		expect(splitKeyMatches(['a', 'b'], 'a.b.c')[0]).toEqual({ text: 'b', match: true });
	});

	it('writes YAML that keeps ambiguous strings as strings', () => {
		const yaml = stringify({ a: 'true', b: '1e-3', c: 'x: y', d: true, e: 0.001 });
		expect(yaml).toBe('a: "true"\nb: "1e-3"\nc: "x: y"\nd: true\ne: 0.001\n');
	});
});
