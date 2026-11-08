// What a failure means for the person looking at the page.

import { ApiError } from '#lib/api/client.js';

/** No answer, or none from the server itself: nothing about the request was refused. */
export function isConnectionFailure(err: unknown): boolean {
	return (
		err instanceof ApiError &&
		(err.status === 0 ||
			err.code === 'storage_unavailable' ||
			err.code === 'bad_response' ||
			(err.status >= 502 && err.status <= 504))
	);
}

/** Likely to pass, or a bug: what's already on screen is still better than an error. */
export function isTemporary(err: unknown): boolean {
	return !(err instanceof ApiError) || err.status >= 500 || isConnectionFailure(err);
}

export type ErrorKind = 'connection' | 'storage' | 'missing' | 'denied' | 'failed';

const capitalize = (s: string) => s[0].toUpperCase() + s.slice(1);
const asSentence = (s: string) => {
	const text = capitalize(s);
	return /[.!?]$/.test(text) ? text : `${text}.`;
};

/** `what` names what was being loaded: "run", "project list". */
export function describeError(
	err: unknown,
	what: string
): { kind: ErrorKind; title: string; message: string } {
	const failed = `Couldn't load the ${what}`;
	if (!(err instanceof ApiError)) {
		return {
			kind: 'failed',
			title: failed,
			message: 'Something went wrong on this page. Try again, or reload it.'
		};
	}
	if (err.code === 'storage_unavailable') {
		return {
			kind: 'storage',
			title: "The server can't reach its database",
			message: "This page will catch up by itself when it's back."
		};
	}
	if (isConnectionFailure(err)) {
		return {
			kind: 'connection',
			title: "Can't reach the server",
			message: `${err.code === 'unreachable' ? '' : `${err.message} `}This page will catch up by itself when it's back.`
		};
	}
	if (err.status === 404) {
		return {
			kind: 'missing',
			title: `${capitalize(what)} not found`,
			message: "It doesn't exist, it was deleted, or you don't have access to it."
		};
	}
	if (err.status === 403) {
		return { kind: 'denied', title: `No access to this ${what}`, message: asSentence(err.message) };
	}
	return { kind: 'failed', title: failed, message: asSentence(err.message) };
}
