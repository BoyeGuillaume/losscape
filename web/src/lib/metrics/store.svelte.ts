// The custom metrics of the project being viewed, shared by its pages: formatting, charts
// and tables read the registry from here, and the Metrics tab edits it (changes apply at
// once and are saved shortly after).

import { toast } from 'svelte-sonner';

import { ApiError, apiJson, apiSend } from '#lib/api/client.js';
import { onRestored, whenConnected } from '#lib/connection.svelte.js';
import { isConnectionFailure } from '#lib/errors.js';
import { errorMessage } from '#lib/notify.js';
import { SaveQueue, typingInField, type SaveStatus } from '#lib/save-queue.js';
import { Registry, type Kind, type MetricDef, type Spec } from './registry.js';

export type { SaveStatus };

type Versioned = { version: number; definitions: MetricDef[] };

class ProjectMetrics {
	project = $state<string | null>(null);
	defs = $state<MetricDef[]>([]);
	registry = $state(new Registry([]));
	/** Of the last edits. */
	status = $state<SaveStatus>('idle');
	private generation = 0;
	/** Of the project being viewed, once its settings are loaded: edits build on those. */
	private queue: SaveQueue<MetricDef[]> | null = null;

	constructor() {
		if (typeof window === 'undefined') return;
		// Edits still waiting go now. Coming back, others may have changed things since.
		window.addEventListener('pagehide', () => this.queue?.flush(true));
		window.addEventListener('pageshow', (e) => e.persisted && this.refresh());
		document.addEventListener('visibilitychange', () => {
			if (document.visibilityState === 'hidden') this.queue?.flush(true);
			else this.refresh();
		});
		onRestored(() => this.refresh());
	}

	private refresh() {
		if (this.project && !typingInField()) void this.load(this.project);
	}

	async load(project: string) {
		const gen = ++this.generation;
		if (project !== this.project) {
			// Edits still waiting belong to the project being left: its queue finishes them.
			this.queue?.flush();
			this.queue = null;
			this.project = project;
			this.status = 'idle';
			this.set([]);
		} else if (this.queue?.busy) {
			return; // what's here is newer than what the server has
		}
		try {
			const { version, definitions } = await apiJson<Versioned>(
				`/projects/${encodeURIComponent(project)}/metric-definitions`
			);
			if (gen !== this.generation) return;
			if (!this.queue) {
				this.queue = this.queueFor(project, version);
				this.set(definitions);
			} else if (version !== this.queue.version) {
				this.queue.version = version;
				this.set(definitions);
			}
		} catch (err) {
			console.warn('losscape: loading metric definitions failed', err);
		}
	}

	private queueFor(project: string, version: number) {
		return new SaveQueue<MetricDef[]>(
			async (definitions, based, closing) => {
				const path = `/projects/${encodeURIComponent(project)}/metric-definitions`;
				const body: Versioned = { version: based, definitions };
				const saved = await apiSend<{ version: number }>('PUT', path, body, { closing });
				return saved.version;
			},
			version,
			{
				status: (status) => {
					if (project === this.project) this.status = status;
				},
				failed: (err) => this.saveFailed(project, err),
				retryAfter: (err) => (isConnectionFailure(err) ? whenConnected() : null)
			}
		);
	}

	private saveFailed(project: string, err: unknown) {
		const stale = err instanceof ApiError && err.status === 409;
		if (project !== this.project) {
			toast.error(`Couldn't save the metric settings of ${project}`, {
				description: stale
					? 'Someone changed them in the meantime, so the last edits there were not kept.'
					: `${errorMessage(err)} The last edits there were not kept.`
			});
			return;
		}
		if (stale) {
			toast.error('Someone else changed these metric settings', {
				description:
					"So your last edit wasn't saved, and their version is shown now. Make it again if it's still needed."
			});
		} else {
			toast.error("Couldn't save the metric settings", {
				description: `${errorMessage(err)} The last saved settings are shown again.`
			});
		}
		// What's on screen isn't what's saved: start over from the server's.
		this.queue = null;
		void this.load(project);
	}

	set(defs: MetricDef[]) {
		this.defs = defs;
		this.registry = new Registry(defs);
	}

	/** A metric's own definition: its description, or its formula if it is computed. */
	own(name: string): MetricDef | undefined {
		return this.defs.find((d) => d.name === name && d.kind !== 'family');
	}

	/** Applies `next` now and saves it shortly after (several quick edits, one save). */
	commit(next: MetricDef[]) {
		// Not loaded yet, or it failed: saving would replace settings this hasn't even seen.
		if (!this.queue) {
			toast.error("The metric settings aren't loaded yet", {
				description: 'Nothing was changed. Try again in a moment.'
			});
			if (this.project) void this.load(this.project);
			return;
		}
		this.generation++; // a load under way is older than this
		this.set(next);
		this.queue.schedule(next);
	}

	/** Changes fields of a metric's own definition (undefined: back to inherited). */
	patch(name: string, change: Partial<Spec>) {
		const current = this.own(name);
		const kind: Kind = current?.kind ?? 'describe';
		const spec: Spec = { ...current?.spec, ...change };
		for (const [k, v] of Object.entries(spec)) {
			if (v === undefined) delete spec[k as keyof Spec];
		}
		// A description with nothing left goes; a computed metric stays.
		const keep = kind !== 'describe' || Object.keys(spec).length > 0;
		this.replace(current ?? null, keep ? { name, kind, spec } : null);
	}

	/** Replaces `old` by `def` (in place), adds `def`, or removes `old`. */
	replace(old: MetricDef | null, def: MetricDef | null) {
		const next = this.defs.filter((d) => d !== old);
		if (def) next.splice(old ? this.defs.indexOf(old) : next.length, 0, def);
		this.commit(next);
	}

	/** Removes a metric added to the project, or its settings: logged data stays. Can be undone. */
	remove(name: string) {
		const def = this.own(name);
		const project = this.project;
		if (!def) return;
		this.replace(def, null);
		toast.success(`${name} removed`, {
			description:
				def.kind === 'formula'
					? 'The computed metric is gone from charts and tables; logged data is unchanged.'
					: 'Its settings are gone; logged data is unchanged.',
			action: {
				label: 'Undo',
				onClick: () => {
					if (this.project === project && !this.defs.some((d) => d.name === name)) {
						this.replace(null, def);
					} else {
						toast.error(`Couldn't bring back ${name}`, {
							description:
								this.project === project
									? 'Another metric has taken its name.'
									: `It belongs to ${project}, and this is ${this.project}.`
						});
					}
				}
			}
		});
	}
}

export const projectMetrics = new ProjectMetrics();

/** Loads the custom metrics of `project()` (call from a page's script). */
export function useProjectMetrics(project: () => string) {
	$effect(() => {
		const p = project();
		if (p) void projectMetrics.load(p);
	});
}

export const currentRegistry = () => projectMetrics.registry;
