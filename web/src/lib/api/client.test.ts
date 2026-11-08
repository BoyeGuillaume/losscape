import { afterEach, describe, expect, it, vi } from 'vitest';

import type { Me } from '#lib/api/types.js';
import { auth } from '#lib/auth.svelte.js';

import { ApiError, apiJson, apiSend } from './client.js';

const answer = (body: string, status = 200, type = 'application/json') =>
	vi.stubGlobal(
		'fetch',
		vi.fn(async (url: string) =>
			String(url).startsWith('/readyz')
				? new Response('ok')
				: new Response(body, { status, headers: { 'content-type': type } })
		)
	);

const failure = async (request: Promise<unknown>) => {
	const err = await request.then(
		() => null,
		(e) => e
	);
	expect(err).toBeInstanceOf(ApiError);
	return err as ApiError;
};

describe('api client', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
		auth.user = undefined;
		auth.expired = false;
	});

	it("keeps the server's own errors", async () => {
		answer('{"code":"unknown_run","message":"no such run"}', 404);
		const err = await failure(apiJson('/runs/x'));
		expect([err.status, err.code, err.message]).toEqual([404, 'unknown_run', 'no such run']);
	});

	it('has something to say about answers from something else', async () => {
		answer('<html>Bad gateway</html>', 502, 'text/html');
		expect((await failure(apiJson('/x'))).message).toBe("The server isn't responding.");
		answer('<html>Too large</html>', 413, 'text/html');
		expect((await failure(apiJson('/x'))).message).toBe('That is too large for the server.');
		answer('', 500, 'text/plain');
		expect((await failure(apiJson('/x'))).message).toContain('500');
	});

	it("doesn't trust a success that isn't JSON: a login page, say", async () => {
		answer('<html>Sign in to the wifi</html>', 200, 'text/html');
		expect((await failure(apiJson('/x'))).code).toBe('bad_response');
		expect((await failure(apiSend('POST', '/x', {}))).code).toBe('bad_response');
	});

	it('words database failures for people', async () => {
		answer('{"code":"storage_unavailable","message":"metric store unavailable"}', 503);
		const err = await failure(apiJson('/x'));
		expect(err.code).toBe('storage_unavailable');
		expect(err.message).toBe("The server can't reach its database.");
	});

	it('reports no connection, but not a request the caller gave up on', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn(async () => Promise.reject(new TypeError('Failed to fetch')))
		);
		const err = await failure(apiJson('/x'));
		expect([err.status, err.code]).toEqual([0, 'unreachable']);
		expect(err.message).not.toContain('fetch');

		const controller = new AbortController();
		controller.abort();
		await expect(apiJson('/x', { signal: controller.signal })).rejects.toBeInstanceOf(TypeError);
	});

	it('notices a session ending, unlike a failed sign-in', async () => {
		answer('{"code":"invalid_credentials","message":"wrong"}', 401);
		auth.user = null;
		await failure(apiSend('POST', '/auth/login', {}));
		expect(auth.expired).toBe(false);

		answer('{"code":"unauthorized","message":"sign in"}', 401);
		auth.user = { username: 'ada' } as Me;
		await failure(apiJson('/projects'));
		expect(auth.user).toBe(null);
		expect(auth.expired).toBe(true);
	});
});
