// Adding and removing tags on runs, with toasts saying what happened.

import { apiSend } from '#lib/api/client.js';
import { withToast } from '#lib/notify.js';
import { projectTags } from './store.svelte.js';

const plural = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`;

/** Adds `add` and removes `remove` on runs; resolves to whether it worked. */
export async function changeTags(
	project: string,
	runIds: string[],
	change: { add?: string[]; remove?: string[] }
): Promise<boolean> {
	const [add, remove] = [change.add ?? [], change.remove ?? []];
	const tag = [...add, ...remove].join(', ');
	const ok = await withToast(
		() =>
			apiSend<{ changed: number }>('POST', `/projects/${encodeURIComponent(project)}/runs/tags`, {
				runs: runIds,
				add,
				remove
			}),
		{
			done: add.length ? `Tagged ${tag}` : `${tag} removed`,
			detail: ({ changed }) =>
				changed === 0
					? `Nothing to change: ${add.length ? 'the runs already had it' : "the runs didn't have it"}.`
					: `${plural(changed, 'run')} changed${changed < runIds.length ? ` (the other ${plural(runIds.length - changed, 'run')} already ${add.length ? 'had it' : "didn't have it"})` : ''}. Filter with tag:${[...add, ...remove][0]}.`,
			failed: add.length ? `Couldn't tag the runs ${tag}` : `Couldn't remove ${tag}`
		}
	);
	projectTags.refresh();
	return ok !== undefined;
}

/** The API path of a tag's style (names may contain slashes). */
export const tagPath = (project: string, name: string) =>
	`/projects/${encodeURIComponent(project)}/tags/${name.split('/').map(encodeURIComponent).join('/')}`;
