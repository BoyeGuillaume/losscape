import { describe, expect, it } from 'vitest';

import type { ModelNode } from '#lib/api/types.js';
import {
	allKeys,
	expandRepeats,
	hasRepeats,
	initiallyExpanded,
	MAX_ROWS,
	modelRows
} from './model-tree.js';

const linear = (name: string, params = 10): ModelNode => ({
	name,
	class: 'Linear',
	params,
	trainable: params,
	tensors: [{ name: 'weight', shape: [2, 5], trainable: true }]
});
const root: ModelNode = {
	name: '',
	class: 'Net',
	params: 30,
	trainable: 30,
	children: [
		{
			name: 'encoder',
			class: 'Encoder',
			params: 20,
			trainable: 20,
			children: [linear('fc1'), linear('fc2')]
		},
		{ ...linear('head'), hidden_children: 3 }
	]
};
const keys = (rows: ReturnType<typeof modelRows>) => rows.map((r) => r.key);

describe('model rows', () => {
	it('opens on the model and what is directly in it', () => {
		const rows = modelRows(root, initiallyExpanded(root));
		expect(keys(rows)).toEqual(['.', '..encoder', '..head']);
		expect(rows.map((r) => r.depth)).toEqual([0, 1, 1]);
		expect(rows.map((r) => r.expanded)).toEqual([true, false, false]);
	});

	it('shows parameters and children when opened', () => {
		const rows = modelRows(root, new Set(['.', '..encoder', '..head']));
		expect(keys(rows)).toEqual([
			'.',
			'..encoder',
			'..encoder.fc1',
			'..encoder.fc2',
			'..head',
			'..head#weight',
			'..head#hidden'
		]);
		expect(rows.at(-1)?.hiddenChildren).toBe(3);
		expect(rows.at(-2)?.tensor?.shape).toEqual([2, 5]);
	});

	it('finds modules by name or class, with what leads to them', () => {
		const byName = modelRows(root, new Set(), 'FC2');
		expect(keys(byName)).toEqual(['.', '..encoder', '..encoder.fc2']);
		const byClass = modelRows(root, new Set(), 'linear');
		expect(byClass.map((r) => r.node?.name)).toEqual(['', 'encoder', 'fc1', 'fc2', 'head']);
		expect(modelRows(root, new Set(), 'nothing')).toEqual([]);
	});

	it('counts the copies of what is collapsed, down the tree', () => {
		const repeated: ModelNode = {
			name: '',
			class: 'Net',
			params: 60,
			trainable: 60,
			children: [
				{
					name: '0',
					class: 'Block',
					params: 20,
					trainable: 20,
					repeat: 3,
					last: '2',
					children: [linear('fc', 20)]
				}
			]
		};
		const rows = modelRows(repeated, new Set(['.', '..0']));
		expect(rows.map((r) => [r.node?.name, r.copies])).toEqual([
			['', 1],
			['0', 3],
			['fc', 3]
		]);
	});

	describe('every layer', () => {
		const block = (over: Partial<ModelNode>): ModelNode => ({
			name: '0',
			class: 'Block',
			params: 20,
			trainable: 20,
			children: [linear('fc', 20)],
			...over
		});
		const collapsed = (over: Partial<ModelNode>): ModelNode => ({
			name: '',
			class: 'Net',
			params: 60,
			trainable: 60,
			children: [block({ repeat: 3, last: '2', ...over })]
		});

		it('turns a collapsed block back into its modules', () => {
			const tree = expandRepeats(collapsed({ names: ['0', '1', '2'] }));
			expect(tree.children?.map((c) => [c.name, c.repeat, c.last, c.names])).toEqual([
				['0', undefined, undefined, undefined],
				['1', undefined, undefined, undefined],
				['2', undefined, undefined, undefined]
			]);
			expect(hasRepeats(tree)).toBe(false);
			expect(tree.params).toBe(60);
			expect(tree.children?.[2].children?.[0].name).toBe('fc');
		});

		it('uses the recorded names, or counts up on older runs', () => {
			const names = (node: ModelNode) => node.children?.map((c) => c.name);
			const gaps = collapsed({ name: '1', names: ['1', '3', '7'] });
			expect(names(expandRepeats(gaps))).toEqual(['1', '3', '7']);
			const older = collapsed({ name: '4' });
			expect(names(expandRepeats(older))).toEqual(['4', '5', '6']);
		});

		it('reaches blocks inside blocks, with one row each', () => {
			const inner = block({ name: '0', repeat: 2, last: '1' });
			const outer = block({
				name: '0',
				repeat: 3,
				last: '2',
				params: 40,
				trainable: 40,
				children: [inner]
			});
			const tree = expandRepeats({ ...collapsed({}), children: [outer] });
			const rows = modelRows(tree, allKeys(tree));
			const blocks = rows.filter((r) => r.node?.class === 'Block');
			expect(blocks).toHaveLength(3 + 3 * 2);
			expect(new Set(rows.map((r) => r.key)).size).toBe(rows.length);
			expect(rows.every((r) => r.copies === 1)).toBe(true);
		});

		it('knows when there is nothing to expand', () => {
			expect(hasRepeats(root)).toBe(false);
			expect(hasRepeats(collapsed({}))).toBe(true);
			expect(expandRepeats(linear('x'))).toEqual(linear('x'));
		});

		it('caps what it draws', () => {
			const huge = expandRepeats(collapsed({ repeat: MAX_ROWS * 2, last: undefined }));
			const rows = modelRows(huge, new Set(['.']));
			expect(rows).toHaveLength(MAX_ROWS + 1);
			expect(rows.at(-1)?.hiddenRows).toBe(MAX_ROWS * 2 + 1 - MAX_ROWS);
		});
	});

	it('knows what can be opened', () => {
		const rows = modelRows(root, new Set(['.', '..encoder']));
		const expandable = rows.filter((r) => r.expandable).map((r) => r.node?.name);
		expect(expandable).toEqual(['', 'encoder', 'fc1', 'fc2', 'head']);
		expect(allKeys(root).size).toBe(5);
		expect(modelRows({ ...linear('x'), tensors: [] }, new Set())[0].expandable).toBe(false);
	});
});
