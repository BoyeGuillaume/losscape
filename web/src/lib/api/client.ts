// HTTP client for the losscape API. The session cookie is sent automatically
// (same origin); a 401 means the session ended.

import { auth } from '#lib/auth.svelte.js';

export class ApiError extends Error {
	readonly status: number;
	readonly code: string;

	constructor(status: number, code: string, message: string) {
		super(message);
		this.status = status;
		this.code = code;
	}
}

export async function apiFetch(path: string, init: RequestInit = {}): Promise<Response> {
	const res = await fetch(`/api/v0${path}`, init);
	if (!res.ok) {
		const body = await res.json().catch(() => ({}));
		if (res.status === 401) auth.user = null;
		throw new ApiError(res.status, body.code ?? 'error', body.message ?? res.statusText);
	}
	return res;
}

export async function apiJson<T>(path: string, init?: RequestInit): Promise<T> {
	return (await apiFetch(path, init)).json() as Promise<T>;
}

/** JSON request with a body (POST/PUT/DELETE). */
export async function apiSend<T = void>(method: string, path: string, body?: unknown): Promise<T> {
	const res = await apiFetch(path, {
		method,
		headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
		body: body === undefined ? undefined : JSON.stringify(body)
	});
	if (res.status === 204) return undefined as T;
	return res.json() as Promise<T>;
}
