// Families found in metric names: metrics that differ only by a number in their last part
// (val/pck-4, val/pck-8, val/pck-16), offered in the Metrics tab as `val/pck-{k}`.

export type FamilyCandidate = { pattern: string; members: string[] };

/** Families of at least `min` metrics, system metrics aside. */
export function detectFamilies(keys: Iterable<string>, min = 3): FamilyCandidate[] {
	const groups = new Map<string, Set<string>>();
	for (const key of keys) {
		if (key.startsWith('system/') || /[*{}]/.test(key)) continue;
		const slash = key.lastIndexOf('/');
		// The last number of the last part: its text before and after stays.
		const m = /^(.*?)(\d+(?:\.\d+)?)(\D*)$/.exec(key.slice(slash + 1));
		if (!m) continue;
		const pattern = `${key.slice(0, slash + 1)}${m[1]}{k}${m[3]}`;
		if (!groups.has(pattern)) groups.set(pattern, new Set());
		groups.get(pattern)!.add(key);
	}
	return [...groups]
		.filter(([, members]) => members.size >= min)
		.map(([pattern, members]) => ({ pattern, members: [...members].sort(byNumber(pattern)) }))
		.sort((a, b) => a.pattern.localeCompare(b.pattern));
}

/** Members in order of their number. */
function byNumber(pattern: string) {
	const at = pattern.indexOf('{k}');
	const num = (key: string) => parseFloat(key.slice(at));
	return (a: string, b: string) => num(a) - num(b);
}
