import { untrack } from 'svelte';

let version = $state(0);

/** Tells every project list in the app to reload (e.g. after creating a project). */
export function projectsChanged() {
	version++;
}

/** Calls `refresh` whenever `projectsChanged()` is called. Call from a component's script. */
export function onProjectsChanged(refresh: () => void) {
	let seen = untrack(() => version);
	$effect(() => {
		if (version === seen) return;
		seen = version;
		untrack(refresh);
	});
}

/** Same rule as the server: 1-100 characters from [A-Za-z0-9._-]. */
export const PROJECT_NAME = /^[A-Za-z0-9._-]{1,100}$/;
