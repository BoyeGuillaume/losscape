// The runs picked in a project: charted on the Runs tab, acted on in bulk (tags, archive),
// and compared. One selection per project, kept in this browser across pages and visits.

const shortId = (id: string) => id.replaceAll('-', '');

class Selection {
	/** Run ids (32 hex, no dashes), in the order picked. */
	ids = $state<string[]>([]);
	/**
	 * A lookup of `ids`, rebuilt when the list changes. Not a `$derived`: the selection
	 * outlives the page that first asked for it, and a derived made there would stop
	 * updating once that page is gone.
	 */
	private lookup: { ids: string[]; set: Set<string> } | null = null;

	constructor(private project: string) {
		try {
			const saved = JSON.parse(localStorage.getItem(this.key) ?? '[]');
			if (Array.isArray(saved)) this.ids = saved.filter((id) => typeof id === 'string');
		} catch {
			// Storage unavailable: the selection lasts as long as the page.
		}
	}

	private get key() {
		return `losscape:selection:${this.project}`;
	}

	get size() {
		return this.ids.length;
	}

	private get set(): Set<string> {
		const ids = this.ids;
		if (this.lookup?.ids !== ids) {
			// eslint-disable-next-line svelte/prefer-svelte-reactivity -- a cache, rebuilt not mutated
			this.lookup = { ids, set: new Set(ids) };
		}
		return this.lookup.set;
	}

	has(id: string) {
		return this.set.has(shortId(id));
	}

	/** Adds (or removes) runs; added ones go last. */
	toggle(ids: string[], on: boolean) {
		const short = ids.map(shortId);
		if (on) this.replace([...this.ids, ...short.filter((id) => !this.set.has(id))]);
		else {
			// eslint-disable-next-line svelte/prefer-svelte-reactivity -- a lookup, never mutated
			const drop = new Set(short);
			this.replace(this.ids.filter((id) => !drop.has(id)));
		}
	}

	replace(ids: string[]) {
		// eslint-disable-next-line svelte/prefer-svelte-reactivity -- only to drop duplicates
		this.ids = [...new Set(ids.map(shortId))];
		try {
			if (this.ids.length) localStorage.setItem(this.key, JSON.stringify(this.ids));
			else localStorage.removeItem(this.key);
		} catch {
			// Not kept across visits.
		}
	}

	clear() {
		this.replace([]);
	}
}

// eslint-disable-next-line svelte/prefer-svelte-reactivity -- a cache; each Selection is reactive
const selections = new Map<string, Selection>();

/** The selection of `project` (the same object on every page). */
export function selectionOf(project: string): Selection {
	let s = selections.get(project);
	if (!s) selections.set(project, (s = new Selection(project)));
	return s;
}

export type { Selection };
