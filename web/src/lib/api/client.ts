// HTTP client for the losscape API. The session cookie is sent automatically
// (same origin); a 401 means the session ended. Whatever goes wrong comes out as an ApiError
// with a message fit to show.

import { auth } from '#lib/auth.svelte.js';
import { suspect } from '#lib/connection.svelte.js';

const KEEPALIVE_LIMIT = 60_000;
/** A server that stopped answering may never close the connection. */
const REQUEST_LIMIT_MS = 60_000;

export class ApiError extends Error {
	readonly status: number;
	readonly code: string;

	constructor(status: number, code: string, message: string) {
		super(message);
		this.status = status;
		this.code = code;
	}
}

/** For a response with no message of ours: from a proxy, or one that isn't JSON. */
function messageFor(res: Response): string {
	if (res.status === 413) return 'That is too large for the server.';
	if (res.status === 429) return 'Too many requests. Wait a moment and try again.';
	if (res.status >= 502 && res.status <= 504) return "The server isn't responding.";
	return `The server answered with an error (${res.status}).`;
}

/** A request can pass through here with its own signal: one for a long-lived stream. */
export async function apiFetch(path: string, init: RequestInit = {}): Promise<Response> {
	let res: Response;
	try {
		res = await fetch(`/api/v0${path}`, {
			...init,
			signal: init.signal ?? AbortSignal.timeout(REQUEST_LIMIT_MS)
		});
	} catch (err) {
		if (init.signal?.aborted) throw err; // the caller changed its mind
		suspect();
		throw err instanceof DOMException && err.name === 'TimeoutError'
			? new ApiError(0, 'timeout', 'The server took too long to answer.')
			: new ApiError(0, 'unreachable', "The server can't be reached.");
	}
	if (!res.ok) {
		const body = await res.json().catch(() => null);
		if (res.status === 401) {
			if (auth.user) auth.expired = true;
			auth.user = null;
		}
		if (res.status >= 502 && res.status <= 504) suspect();
		if (body?.code === 'storage_unavailable') {
			throw new ApiError(res.status, body.code, "The server can't reach its database.");
		}
		throw new ApiError(res.status, body?.code ?? 'error', body?.message ?? messageFor(res));
	}
	return res;
}

/** The body of an answer: something else than us may have answered, like a login page. */
async function readJson<T>(res: Response): Promise<T> {
	try {
		return (await res.json()) as T;
	} catch (err) {
		if (err instanceof DOMException && err.name === 'AbortError') throw err;
		suspect();
		throw new ApiError(
			res.status,
			'bad_response',
			"The server sent an answer this page can't read. Try reloading it."
		);
	}
}

export async function apiJson<T>(path: string, init?: RequestInit): Promise<T> {
	return readJson<T>(await apiFetch(path, init));
}

/** JSON request with a body (POST/PUT/DELETE). */
export async function apiSend<T = void>(
	method: string,
	path: string,
	body?: unknown,
	options: { closing?: boolean } = {}
): Promise<T> {
	const text = body === undefined ? undefined : JSON.stringify(body);
	const res = await apiFetch(path, {
		method,
		headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
		body: text,
		// Lets the request outlive the page. Browsers refuse it for larger bodies.
		keepalive: options.closing && (text?.length ?? 0) < KEEPALIVE_LIMIT
	});
	if (res.status === 204) return undefined as T;
	return readJson<T>(res);
}
