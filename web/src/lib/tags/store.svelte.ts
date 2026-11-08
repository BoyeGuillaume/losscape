// The tags of the project being viewed (colors, descriptions, how many runs have them),
// shared by its pages.

import { apiJson } from '#lib/api/client.js';
import { onRestored } from '#lib/connection.svelte.js';
import type { TagInfo } from '#lib/api/types.js';
import { colorFromName, isTagColor, type TagColor } from './palette.js';

class ProjectTags {
	project = $state<string | null>(null);
	list = $state<TagInfo[]>([]);
	private byName = $derived(new Map(this.list.map((t) => [t.name, t])));
	private generation = 0;

	async load(project: string) {
		const gen = ++this.generation;
		if (project !== this.project) {
			this.project = project;
			this.list = [];
		}
		try {
			const list = await apiJson<TagInfo[]>(`/projects/${encodeURIComponent(project)}/tags`);
			if (gen === this.generation) this.list = list;
		} catch (err) {
			console.warn('losscape: loading tags failed', err);
		}
	}

	/** Reloads after tags changed (added, removed, described). */
	refresh() {
		if (this.project) void this.load(this.project);
	}

	info(name: string): TagInfo | undefined {
		return this.byName.get(name);
	}

	color(name: string): TagColor {
		const c = this.byName.get(name)?.color;
		return isTagColor(c) ? c : colorFromName(name);
	}
}

export const projectTags = new ProjectTags();
onRestored(() => projectTags.refresh());

/** Loads the tags of `project()` (call from a page's script). */
export function useProjectTags(project: () => string) {
	$effect(() => {
		const p = project();
		if (p) void projectTags.load(p);
	});
}
