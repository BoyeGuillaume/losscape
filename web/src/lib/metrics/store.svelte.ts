// The custom metrics of the project being viewed, shared by its pages: formatting, charts
// and tables read the registry from here, and the Metrics tab edits it (changes apply at
// once and are saved shortly after).

import { toast } from 'svelte-sonner';

import { apiJson, apiSend } from '#lib/api/client.js';
import { errorMessage } from '#lib/notify.js';
import { Registry, type Kind, type MetricDef, type Spec } from './registry.js';

export type SaveStatus = 'idle' | 'pending' | 'saving' | 'saved' | 'error';

class ProjectMetrics {
	project = $state<string | null>(null);
	defs = $state<MetricDef[]>([]);
	registry = $state(new Registry([]));
	/** Of the last edits. */
	status = $state<SaveStatus>('idle');
	private generation = 0;
	private timer: ReturnType<typeof setTimeout> | undefined;

	async load(project: string) {
		const gen = ++this.generation;
		if (project !== this.project) {
			// Edits waiting to be saved belong to the project being left.
			if (this.status === 'pending') {
				clearTimeout(this.timer);
				void this.save();
			}
			this.project = project;
			this.status = 'idle';
			this.set([]);
		} else if (this.status === 'pending' || this.status === 'saving') {
			return; // what's here is newer than what the server has
		}
		try {
			const defs = await apiJson<MetricDef[]>(
				`/projects/${encodeURIComponent(project)}/metric-definitions`
			);
			if (gen === this.generation) this.set(defs);
		} catch (err) {
			console.warn('losscape: loading metric definitions failed', err);
		}
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
		this.generation++; // a load under way is older than this
		this.set(next);
		this.status = 'pending';
		clearTimeout(this.timer);
		this.timer = setTimeout(() => void this.save(), 500);
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

	private async save() {
		const project = this.project;
		if (!project) return;
		const defs = this.defs;
		this.status = 'saving';
		try {
			await apiSend('PUT', `/projects/${encodeURIComponent(project)}/metric-definitions`, defs);
			if (this.status === 'saving') this.status = 'saved';
		} catch (err) {
			if (project !== this.project) {
				toast.error(`Couldn't save the metric settings of ${project}`, {
					description: `${errorMessage(err)} The last edits there were not kept.`
				});
				return;
			}
			this.status = 'error';
			toast.error("Couldn't save the metric settings", {
				description: `${errorMessage(err)} The last saved settings are shown again.`
			});
			void this.load(project);
		}
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
