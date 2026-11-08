// The custom metrics of the project being viewed, shared by its pages: formatting, charts
// and tables read the registry from here.

import { apiJson } from '#lib/api/client.js';
import { Registry, type MetricDef } from './registry.js';

class ProjectMetrics {
	project = $state<string | null>(null);
	defs = $state<MetricDef[]>([]);
	registry = $state(new Registry([]));
	private generation = 0;

	async load(project: string) {
		const gen = ++this.generation;
		if (project !== this.project) {
			this.project = project;
			this.set([]);
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
