// A project's run list, kept current cheaply: the newest runs come first (the page shows
// them while the rest loads, page by page), then only runs changed since are fetched.

import { apiJson } from '#lib/api/client.js';
import type { RunSummary, RunsPage } from '#lib/api/types.js';
import { onRestored } from '#lib/connection.svelte.js';

const FIRST_PAGE = 500;
const PAGE = 5_000;
/** Runs deleted for good (purged archives) only disappear with a full reload. */
const FULL_RELOAD_MS = 10 * 60_000;

export class RunList {
	/** Null until the first page has loaded. */
	runs = $state.raw<RunSummary[] | null>(null);
	/** More pages are loading. */
	partial = $state(false);
	error = $state<unknown>(null);

	// eslint-disable-next-line svelte/prefer-svelte-reactivity -- bookkeeping; `runs` is published
	private byId = new Map<string, RunSummary>();
	private asOf = 0;
	private lastFull = 0;
	private stopped = false;
	private timer: ReturnType<typeof setTimeout> | undefined;
	private busy = false;
	private unlisten: (() => void) | undefined;

	constructor(
		/** `/projects/<name>` */
		private path: string,
		/** Archived runs only, or only the others. */
		private archived: boolean,
		/** Time between refreshes, given the number of runs. */
		private everyMs: (count: number) => number
	) {}

	start() {
		void this.reload(true);
		this.unlisten = onRestored(() => {
			if (this.runs === null && !this.busy) void this.reload(true);
			else this.refresh();
		});
	}

	stop() {
		this.stopped = true;
		clearTimeout(this.timer);
		this.unlisten?.();
	}

	/** Fetches what changed now (after an action on runs). */
	refresh() {
		void this.update();
	}

	private url(params: Record<string, string | number>) {
		// eslint-disable-next-line svelte/prefer-svelte-reactivity -- built once for a request
		const query = new URLSearchParams({
			...(this.archived ? { archived: 'true' } : {}),
			...Object.fromEntries(Object.entries(params).map(([k, v]) => [k, String(v)]))
		});
		return `${this.path}/runs?${query}`;
	}

	/** Loads every run page by page; the first time, shows each page as it arrives. */
	private async reload(progressive: boolean) {
		this.busy = true;
		try {
			let page = await apiJson<RunsPage>(this.url({ limit: FIRST_PAGE }));
			if (this.stopped) return;
			// eslint-disable-next-line svelte/prefer-svelte-reactivity -- bookkeeping, as above
			const byId = new Map(page.runs.map((r) => [r.id, r]));
			this.asOf = page.as_of_ms;
			if (progressive) {
				this.byId = byId;
				this.partial = page.next !== null;
				this.publish();
			}
			while (page.next) {
				page = await apiJson<RunsPage>(this.url({ limit: PAGE, after: page.next }));
				if (this.stopped) return;
				for (const r of page.runs) byId.set(r.id, r);
				if (progressive) this.publish();
			}
			this.byId = byId;
			this.partial = false;
			this.lastFull = Date.now();
			this.error = null;
			this.publish();
		} catch (err) {
			if (this.runs === null) this.error = err;
			else console.warn('losscape: loading runs failed', err);
		} finally {
			this.busy = false;
			this.schedule();
		}
		// Changes made while the pages loaded.
		if (!this.stopped) void this.update();
	}

	/** Merges the runs changed since the last listing. */
	private async update() {
		if (this.busy || this.stopped || this.runs === null) return;
		this.busy = true;
		try {
			const page = await apiJson<RunsPage>(this.url({ since_ms: this.asOf }));
			if (this.stopped) return;
			this.asOf = page.as_of_ms;
			for (const r of page.runs) {
				// Archived or restored meanwhile: it moves to the other list.
				if ((r.archived_at_ms !== null) === this.archived) this.byId.set(r.id, r);
				else this.byId.delete(r.id);
			}
			if (page.runs.length) this.publish();
		} catch (err) {
			console.warn('losscape: refreshing runs failed', err);
		} finally {
			this.busy = false;
		}
	}

	private schedule() {
		clearTimeout(this.timer);
		if (this.stopped) return;
		this.timer = setTimeout(async () => {
			if (Date.now() - this.lastFull > FULL_RELOAD_MS) await this.reload(false);
			else {
				await this.update();
				this.schedule();
			}
		}, this.everyMs(this.byId.size));
	}

	private publish() {
		this.runs = [...this.byId.values()];
	}
}
