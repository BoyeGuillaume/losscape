// A model's modules as the rows on screen: what's expanded, or what a search finds.

import type { ModelNode, ModelTensor } from '#lib/api/types.js';

export type ModelRow = {
	/** Which module it is, for opening and closing it. */
	key: string;
	/** Unique among the rows, whatever names a model came with. */
	id: string;
	depth: number;
	node?: ModelNode;
	tensor?: ModelTensor;
	expandable: boolean;
	expanded: boolean;
	/** Children that a very large model left out. */
	hiddenChildren?: number;
	/** How many of it there are, with what's collapsed around it (`Block × 3`). */
	copies: number;
	/** Rows left out of a list too long to draw. */
	hiddenRows?: number;
};

/** Rows drawn at most: every layer of a very large model, all open, would freeze the page. */
export const MAX_ROWS = 5_000;
/** How far to look for what's left out, and for what to open: the numbers in a model's
 * description come from whoever logged it, and a block of a billion must not freeze the page. */
const MAX_VISITED = 200_000;

export function hasRepeats(node: ModelNode): boolean {
	return !!node.repeat || (node.children ?? []).some(hasRepeats);
}

/**
 * The names of a block's copies, on runs that didn't record them: counting up from the first,
 * by the number it ends with, or else after its name.
 */
function countedNames(name: string, count: number): string[] {
	const numbered = /^(.*?)(\d+)$/.exec(name);
	const n = Math.min(count, MAX_VISITED);
	return Array.from({ length: n }, (_, i) =>
		numbered ? `${numbered[1]}${Number(numbered[2]) + i}` : i ? `${name}#${i + 1}` : name
	);
}

/** The tree with collapsed neighbours as the separate modules they are. */
export function expandRepeats(node: ModelNode): ModelNode {
	const children = node.children?.flatMap((child) => {
		const open = expandRepeats(child);
		if (!open.repeat) return [open];
		const names = (open.names ?? countedNames(child.name, open.repeat)).slice(0, MAX_VISITED);
		const one = { ...open, repeat: undefined, last: undefined, names: undefined };
		return names.map((name) => ({ ...one, name }));
	});
	return children ? { ...node, children } : node;
}

const childKey = (key: string, name: string) => (key ? `${key}.${name}` : name || '.');

/** Modules open at first: the model and what's directly in it. */
export function initiallyExpanded(root: ModelNode): Set<string> {
	return new Set([childKey('', root.name)]);
}

function matches(node: ModelNode, query: string): boolean {
	return node.name.toLowerCase().includes(query) || node.class.toLowerCase().includes(query);
}

/** Rows to show. With a query: the modules matching, with what leads to them, all open. */
export function modelRows(root: ModelNode, expanded: ReadonlySet<string>, query = ''): ModelRow[] {
	const q = query.trim().toLowerCase();
	const rows: ModelRow[] = [];

	function found(node: ModelNode): boolean {
		return matches(node, q) || (node.children ?? []).some(found);
	}

	function visit(node: ModelNode, key: string, depth: number, parentCopies: number) {
		if (rows.length >= MAX_VISITED) return;
		const copies = parentCopies * (node.repeat ?? 1);
		const children = node.children ?? [];
		const tensors = node.tensors ?? [];
		const open = q ? true : expanded.has(key);
		rows.push({
			key,
			id: key,
			depth,
			node,
			expandable: children.length + tensors.length + (node.hidden_children ?? 0) > 0,
			expanded: open,
			copies
		});
		if (!open) return;
		if (!q) {
			for (const tensor of tensors) {
				rows.push({
					key: `${key}#${tensor.name}`,
					id: '',
					depth: depth + 1,
					tensor,
					expandable: false,
					expanded: false,
					copies
				});
			}
		}
		for (const child of children) {
			if (!q || found(child)) visit(child, childKey(key, child.name), depth + 1, copies);
		}
		if (node.hidden_children && !q) {
			rows.push({
				key: `${key}#hidden`,
				id: '',
				depth: depth + 1,
				expandable: false,
				expanded: false,
				hiddenChildren: node.hidden_children,
				copies
			});
		}
	}

	if (!q || found(root)) visit(root, childKey('', root.name), 0, 1);
	if (rows.length <= MAX_ROWS) return withIds(rows);
	const hiddenRows = rows.length - MAX_ROWS;
	rows.length = MAX_ROWS;
	rows.push({
		key: '#more',
		id: '#more',
		depth: 0,
		expandable: false,
		expanded: false,
		copies: 1,
		hiddenRows
	});
	return withIds(rows);
}

/** Two modules can have the same name in a description that came from elsewhere. */
function withIds(rows: ModelRow[]): ModelRow[] {
	const seen = new Set<string>();
	for (const row of rows) {
		let id = row.key;
		for (let copy = 2; seen.has(id); copy++) id = `${row.key}~${copy}`;
		row.id = id;
		seen.add(id);
	}
	return rows;
}

/** Every module that can be opened. */
export function allKeys(root: ModelNode): Set<string> {
	const keys = new Set<string>();
	const visit = (node: ModelNode, key: string) => {
		if (keys.size >= MAX_VISITED) return;
		keys.add(key);
		for (const child of node.children ?? []) visit(child, childKey(key, child.name));
	};
	visit(root, childKey('', root.name));
	return keys;
}
