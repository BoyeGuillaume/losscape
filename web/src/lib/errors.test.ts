import { describe, expect, it } from 'vitest';

import { ApiError } from '#lib/api/client.js';

import { describeError, isConnectionFailure, isTemporary } from './errors.js';

const err = (status: number, code = 'error', message = 'no such thing') =>
	new ApiError(status, code, message);

describe('errors', () => {
	it('tells a lost connection from a refusal', () => {
		for (const lost of [
			err(0, 'unreachable'),
			err(0, 'timeout'),
			err(502),
			err(503),
			err(504),
			err(503, 'storage_unavailable'),
			err(200, 'bad_response')
		]) {
			expect(isConnectionFailure(lost)).toBe(true);
			expect(isTemporary(lost)).toBe(true);
		}
		for (const refused of [err(400), err(401), err(403), err(404), err(409)]) {
			expect(isConnectionFailure(refused)).toBe(false);
			expect(isTemporary(refused)).toBe(false);
		}
		expect(isConnectionFailure(err(500, 'internal'))).toBe(false);
		expect(isTemporary(err(500, 'internal'))).toBe(true);
		expect(isConnectionFailure(new TypeError('a bug'))).toBe(false);
		expect(isTemporary(new TypeError('a bug'))).toBe(true);
	});

	it('says what it means, not what the program saw', () => {
		const kind = (e: unknown) => describeError(e, 'run').kind;
		expect(kind(err(0, 'unreachable'))).toBe('connection');
		expect(kind(err(503, 'storage_unavailable'))).toBe('storage');
		expect(kind(err(403))).toBe('denied');
		expect(kind(err(500, 'internal'))).toBe('failed');
		expect(describeError(err(404), 'run').title).toBe('Run not found');
		expect(describeError(err(0, 'unreachable'), 'run').message).not.toContain('no such thing');
		expect(describeError(err(0, 'timeout', 'Too slow.'), 'run').message).toContain('Too slow.');
		expect(describeError(err(400, 'x', 'bad filter'), 'run list')).toMatchObject({
			title: "Couldn't load the run list",
			message: 'Bad filter.'
		});
		// Nothing from inside the program reaches the page.
		const bug = describeError(new TypeError("reading 'x' of undefined"), 'page');
		expect(bug.message).not.toContain('undefined');
	});
});
