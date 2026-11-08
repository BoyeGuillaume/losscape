import type { HandleClientError } from '@sveltejs/kit/hooks';

import { suspect } from '#lib/connection.svelte.js';

/** A page failed to load: most often its code, for lack of a connection. */
export const handleError: HandleClientError = ({ kind, error }) => {
	if (kind !== 'unknown') return;
	console.error('losscape: loading the page failed', error);
	suspect();
};
