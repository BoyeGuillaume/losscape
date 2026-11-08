/** Items by a key, in order of first appearance. Any string is fine as a key: `__proto__` too. */
export function groupBy<T>(items: Iterable<T>, key: (item: T) => string | null): [string, T[]][] {
	const groups = new Map<string, T[]>();
	for (const item of items) {
		const group = key(item);
		if (group === null) continue;
		const members = groups.get(group);
		if (members) members.push(item);
		else groups.set(group, [item]);
	}
	return [...groups];
}

/**
 * A value of an object keyed by data, like a run's summary. Plain indexing also finds what every
 * object inherits: a metric named `constructor` would get a function.
 */
export function own<T>(object: Record<string, T> | null | undefined, key: string): T | undefined {
	return object && Object.hasOwn(object, key) ? object[key] : undefined;
}
