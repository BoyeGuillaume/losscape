import { afterEach, describe, expect, it, vi } from 'vitest';

import { findLibraries, readLibraries, safeLink, type Library } from './libraries.js';

const lib = (name: string, license: string): Library => ({
	name,
	version: '1.0.0',
	license,
	repository: null
});
const LIBRARIES = [lib('tokio', 'MIT'), lib('ring', 'Apache-2.0 AND ISC'), lib('svelte', 'MIT')];

describe('libraries', () => {
	afterEach(() => void vi.unstubAllGlobals());

	it('finds by name or license, whatever the case', () => {
		const names = (q: string) => findLibraries(LIBRARIES, q).map((l) => l.name);
		expect(names('')).toEqual(['tokio', 'ring', 'svelte']);
		expect(names(' TOK ')).toEqual(['tokio']);
		expect(names('isc')).toEqual(['ring']);
		expect(names('mit')).toEqual(['tokio', 'svelte']);
		expect(names('nothing')).toEqual([]);
	});

	it('only links to https', () => {
		expect(safeLink('https://github.com/a/b')).toBe('https://github.com/a/b');
		for (const bad of ['javascript:alert(1)', 'http://a.b', 'data:text/html,x', '', null]) {
			expect(safeLink(bad)).toBe(null);
		}
	});

	it("knows when a build has no list: unknown paths get the app's page", async () => {
		const answer = (type: string, status = 200, body = '{"packages":[]}') =>
			vi.stubGlobal(
				'fetch',
				vi.fn(async () => new Response(body, { status, headers: { 'content-type': type } }))
			);
		answer('application/json');
		expect(await readLibraries('/x.json')).toEqual({ packages: [] });
		answer('text/html');
		expect(await readLibraries('/x.json')).toBe(null);
		answer('application/json', 404);
		expect(await readLibraries('/x.json')).toBe(null);
		answer('application/json', 200, '{"other":1}');
		expect(await readLibraries('/x.json')).toBe(null);
		answer('application/json', 200, 'not json');
		expect(await readLibraries('/x.json')).toBe(null);
		vi.stubGlobal(
			'fetch',
			vi.fn(async () => Promise.reject(new Error('offline')))
		);
		expect(await readLibraries('/x.json')).toBe(null);
	});
});
